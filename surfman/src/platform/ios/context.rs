// surfman/surfman/src/platform/macos/cgl/context.rs
//
//! Wrapper for Core OpenGL contexts.

use std::cell::RefCell;
use std::default::Default;
use super::surface::{Surface, SurfaceTexture};
use super::device::Device;
use crate::info::GLVersion;
use crate::context::{ContextID, CREATE_CONTEXT_MUTEX};
use crate::{ContextAttributes, Error, WindowingApiError, Gl, SurfaceInfo};
use crate::surface::Framebuffer;

use glutin_gles2_sys as ffi;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use core_foundation::base::TCFType;
use core_foundation::bundle::CFBundleGetBundleWithIdentifier;
use core_foundation::bundle::CFBundleGetFunctionPointerForName;
use core_foundation::bundle::CFBundleRef;
use core_foundation::string::CFString;
use std::mem;
use std::os::raw::c_void;
use std::str::FromStr;

static OPENGLES_FRAMEWORK_IDENTIFIER: &'static str = "com.apple.opengles";
thread_local! {
    static OPENGLES_FRAMEWORK: CFBundleRef = {
        unsafe {
            let framework_identifier: CFString =
                FromStr::from_str(OPENGLES_FRAMEWORK_IDENTIFIER).unwrap();
            let framework =
                CFBundleGetBundleWithIdentifier(framework_identifier.as_concrete_TypeRef());
            assert!(!framework.is_null());
            framework
        }
    };
}

pub struct Context {
    pub(crate) id: ContextID,
    pub(crate) eagl_context: ffi::id,
    pub(crate) gl_version: GLVersion,
    framebuffer: Framebuffer<Surface, ()>,
}

impl Drop for Context {
    fn drop(&mut self) {
        let _: () = unsafe { msg_send![self.eagl_context, release] };
    }
}

impl Context {

    pub unsafe fn create_context(mut version: ffi::NSUInteger, descriptor: &ContextDescriptor) -> Result<Context, Error> {
        let context_class = Class::get("EAGLContext").expect("Failed to get class `EAGLContext`");
        let eagl_context: ffi::id = msg_send![context_class, alloc];
        let mut valid_context = ffi::nil;
        while valid_context == ffi::nil && version > 0 {
            valid_context = msg_send![eagl_context, initWithAPI: version];
            version -= 1;
        }
        
        if valid_context == ffi::nil {
            info!("Could not create context with gl version: {:?}", descriptor.gl_version);
            Err(Error::Failed)
        } else {
            info!("Creating context with gl version: {:?}", version);
            let mut next_context_id = CREATE_CONTEXT_MUTEX.lock().unwrap();
            let ctx = Context {
                id: *next_context_id,
                eagl_context: valid_context,
                gl_version: descriptor.gl_version,
                framebuffer: Framebuffer::None,
            };

            next_context_id.0 += 1;
            Ok(ctx)
        }        
    }
    pub unsafe fn make_current(&self) -> Result<(), Error> {
        info!("Make current: {:?}", self.id);
        let context_class = Class::get("EAGLContext").expect("Failed to get class `EAGLContext`");
        let res: BOOL = msg_send![context_class, setCurrentContext: self.eagl_context];
        if res == YES {
            Ok(())
        } else {
            warn!("`EAGLContext setCurrentContext` failed");
            Err(Error::Failed)
        }        
    }

    pub unsafe fn make_no_context_current() -> Result<(), Error> {
        info!("Make no context current");

        let context_class = Class::get("EAGLContext").expect("Failed to get class `EAGLContext`");
        let res: BOOL = msg_send![context_class, setCurrentContext: ffi::nil];
        if res == YES {
            Ok(())
        } else {
            warn!("`EAGLContext setCurrentContext` failed");
            Err(Error::Failed)
        }
    }

    pub fn get_proc_address(&self, symbol_name: &str) -> *const c_void {
        OPENGLES_FRAMEWORK.with(|framework| unsafe {
            let symbol_name: CFString = FromStr::from_str(symbol_name).unwrap();
            CFBundleGetFunctionPointerForName(*framework, symbol_name.as_concrete_TypeRef())
        })
    }    
}

pub struct ContextDescriptor {
    pub(crate) gl_version: GLVersion,
    pub(crate) attribs: ContextAttributes,
}

pub struct NativeContext();

impl Device {

    pub fn create_context(
        &self,
        descriptor: &ContextDescriptor,
        share_with: Option<&Context>,
    ) -> Result<Context, Error> {
        let version = descriptor.gl_version.major;
        let version = version as ffi::NSUInteger;
        if version >= ffi::kEAGLRenderingAPIOpenGLES1 && version <= ffi::kEAGLRenderingAPIOpenGLES3 {
            let ctx = unsafe { Context::create_context(version, descriptor)? };
            Ok(ctx)
        } else {
            warn!(
                "Specified OpenGL ES version ({:?}) is not availble on iOS. Only 1, 2, and 3 are valid options",
                version,
            );
            Err(Error::Failed)
        }        
    }

    pub fn create_context_descriptor(
        &self,
        attributes: &ContextAttributes,
    ) -> Result<ContextDescriptor, Error> {
        Ok(ContextDescriptor{
            gl_version: attributes.version,
            attribs: attributes.clone(),           
        })
    }

    
    pub fn create_context_from_native_context(
        &self,
        native_context: NativeContext,
    ) -> Result<Context, Error> {
        let attribs = ContextAttributes::zeroed();
        let ctx = ContextDescriptor{
            gl_version: attribs.version,
            attribs: attribs,            
        };

        self.create_context(&ctx, None)        
    }

    
    pub fn destroy_context(&self, context: &mut Context) -> Result<(), Error> {
        drop(context);
        Ok(())
    }

    
    pub fn context_descriptor(&self, context: &Context) -> ContextDescriptor {
        ContextDescriptor{
            gl_version: context.gl_version,
            attribs: ContextAttributes::zeroed(),
        }
    }

    pub fn make_context_current(&self, context: &Context) -> Result<(), Error> {        
        unsafe { context.make_current(); }
        Ok(())
    }

    pub fn make_no_context_current(&self) -> Result<(), Error> {
        unsafe { Context::make_no_context_current(); }
        Ok(())
    }

    pub fn context_descriptor_attributes(
        &self, 
        context_descriptor: &ContextDescriptor
    ) -> ContextAttributes {
        context_descriptor.attribs
    }

    pub fn bind_surface_to_context(
         &self,
         context: &mut Context,
         new_surface: Surface,
     ) -> Result<(), (Error, Surface)> {
        match context.framebuffer {
            Framebuffer::External(_) => return Err((Error::ExternalRenderTarget, new_surface)),
            Framebuffer::Surface(_) => return Err((Error::SurfaceAlreadyBound, new_surface)),
            Framebuffer::None => {}
        }

        if new_surface.context_id != context.id {
            return Err((Error::IncompatibleSurface, new_surface));
        }

        context.framebuffer = Framebuffer::Surface(new_surface);
        Ok(())
    }

    pub fn unbind_surface_from_context(
         &self,
         context: &mut Context,
     ) -> Result<Option<Surface>, Error> {
         match context.framebuffer {
            Framebuffer::External(_) => return Err(Error::ExternalRenderTarget),
            Framebuffer::None | Framebuffer::Surface(_) => {}
        }

        match mem::replace(&mut context.framebuffer, Framebuffer::None) {
            Framebuffer::External(_) => unreachable!(),
            Framebuffer::None => Ok(None),
            Framebuffer::Surface(surface) => Ok(Some(surface)),            
        }
     }

    pub fn get_proc_address(&self, context: &Context, symbol_name: &str) -> *const c_void {
        context.get_proc_address(symbol_name)
    }

    /// Returns a unique ID representing a context.
    ///
    /// This ID is unique to all currently-allocated contexts. If you destroy a context and create
    /// a new one, the new context might have the same ID as the destroyed one.
    #[inline]
    pub fn context_id(&self, context: &Context) -> ContextID {
        context.id
    }

    /// Returns various information about the surface attached to a context.
    ///
    /// This includes, most notably, the OpenGL framebuffer object needed to render to the surface.
    pub fn context_surface_info(&self, context: &Context) -> Result<Option<SurfaceInfo>, Error> {
        match context.framebuffer {
            Framebuffer::None => Ok(None),
            Framebuffer::External(_) => Err(Error::ExternalRenderTarget),
            Framebuffer::Surface(ref surface) => Ok(Some(self.surface_info(surface))),
        }
    }

    pub fn native_context(&self, context: &Context) -> NativeContext {
        NativeContext()       
    }    
    
}