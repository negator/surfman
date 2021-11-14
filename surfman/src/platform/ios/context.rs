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
use glutin::dpi::PhysicalSize;
use glutin::event_loop::EventLoop;
use glutin::{
    Context as GlutinContext, ContextBuilder, ContextError, ContextCurrentState, CreationError, GlProfile, GlRequest, NotCurrent,
    RawContext, PossiblyCurrent
};
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

#[derive(Debug)]
#[allow(non_camel_case_types)]
pub enum ContextInner {
    NoGlutinContext,
    Not(GlutinContext<NotCurrent>),
    Possibly(GlutinContext<PossiblyCurrent>),
}

impl Default for ContextInner {
    fn default() -> ContextInner {
        ContextInner::NoGlutinContext
    }
}


pub struct Context {
    pub(crate) id: ContextID,
    pub(crate) glutin_ctx: RefCell<ContextInner>,
    pub(crate) gl_version: GLVersion,
    framebuffer: Framebuffer<Surface, ()>,
}

impl Context {
    pub unsafe fn make_current(&self) {
        println!("Make current: {:?}", self.id);
        let inner = self.glutin_ctx.take();
        let context = match inner {
            ContextInner::Possibly(c) => c.make_current().unwrap(),
            ContextInner::Not(c) => c.make_current().unwrap(),
            NoGlutinContext => panic!("Context not current"),
        };
        println!("Made current: {:?}", context);
        self.glutin_ctx.replace(ContextInner::Possibly(context));
    }

    pub unsafe fn make__not_current(&self) {
        println!("Make not current: {:?}", self.id);
        let inner = self.glutin_ctx.take();
        let context = match inner {            
            ContextInner::Possibly(c) => c.make_not_current().unwrap(),
            ContextInner::Not(c) => c.make_not_current().unwrap(),
            NoGlutinContext => panic!("Context not current"),
        };
        self.glutin_ctx.replace(ContextInner::Not(context));
    }

    pub fn get_proc_address(&self, symbol_name: &str) -> *const c_void {
        let inner = self.glutin_ctx.take();
        let c = match inner {
            ContextInner::Possibly(c) => {
                let addr =  c.get_proc_address(symbol_name);
                self.glutin_ctx.replace(ContextInner::Possibly(c));
                addr
            },
            _                         => panic!("Context not current: {:?}", inner),
        };        
        c
    }

    // pub fn get_current_context(&self) -> Option<&WindowedContext<PossiblyCurrent>> {
    //     match *self.inner {
    //         ContextInner::Possibly(ref c) => Some(c),
    //         _ => None,
    //     }
    // }
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
        let mut next_context_id = CREATE_CONTEXT_MUTEX.lock().unwrap();
        let cb = ContextBuilder::new().with_gl_profile(GlProfile::Core).with_gl(GlRequest::Latest);
        let size_one = PhysicalSize::new(1, 1);
        let el = EventLoop::new();
        let gl_ctx = match cb.build_headless(&el, size_one) {
            Ok(ctx) => Ok(ctx),
            err => Err(Error::Failed),
        }?;
    
        let ctx = Context {
            id: *next_context_id,
            glutin_ctx: RefCell::new(ContextInner::Not(gl_ctx)),
            gl_version: descriptor.gl_version,
            framebuffer: Framebuffer::None,
        };

        next_context_id.0 += 1;
        Ok(ctx)
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
        // context.make__not_current();
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