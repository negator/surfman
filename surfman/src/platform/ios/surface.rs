// surfman/surfman/src/platform/macos/system/surface.rs
//
//! Surface management for macOS.

use super::context::Context;
use super::device::Device;
use super::ffi::{kCVPixelFormatType_32BGRA, kIOMapDefaultCache, IOSurfaceLock, IOSurfaceUnlock};
use super::ffi::{kIOMapWriteCombineCache};
use super::ffi::{IOSurfaceGetAllocSize, IOSurfaceGetBaseAddress, IOSurfaceGetBytesPerRow};
use crate::{gl, Error, SurfaceAccess, SurfaceID, SurfaceType, SurfaceInfo};
use crate::context::ContextID;

use crate::gl::types::{GLenum, GLint, GLuint};
use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use euclid::default::Size2D;
use io_surface::{self, kIOSurfaceBytesPerElement, kIOSurfaceBytesPerRow, IOSurface, IOSurfaceRef};
use io_surface::{kIOSurfaceCacheMode, kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth};
use mach::kern_return::KERN_SUCCESS;
use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::mem;
use std::slice;
use std::thread;

const BYTES_PER_PIXEL: i32 = 4;

const SURFACE_GL_TEXTURE_TARGET: GLenum = gl::TEXTURE_RECTANGLE;

/// Represents a hardware buffer of pixels that can be rendered to via the CPU or GPU and either
/// displayed in a native widget or bound to a texture for reading.
///
/// Surfaces come in two varieties: generic and widget surfaces. Generic surfaces can be bound to a
/// texture but cannot be displayed in a widget (without using other APIs such as Core Animation,
/// DirectComposition, or XPRESENT). Widget surfaces are the opposite: they can be displayed in a
/// widget but not bound to a texture.
///
/// Depending on the platform, each surface may be internally double-buffered.
///
/// Surfaces must be destroyed with the `destroy_surface()` method, or a panic will occur.
///
pub struct Surface {
    pub(crate) context_id: ContextID,
    pub(crate) io_surface: IOSurface,
    pub(crate) size: Size2D<i32>,
    access: SurfaceAccess,
    pub(crate) destroyed: bool,
}

#[derive(Debug)]
pub struct SurfaceTexture {
    pub(crate) surface: Surface,
    pub(crate) texture_object: GLuint,
    pub(crate) phantom: PhantomData<*const ()>,
}

/// A wrapper around an `IOSurface`.
#[derive(Clone)]
pub struct NativeSurface(pub IOSurfaceRef);

#[derive(Clone)]
pub struct NativeWidget();

unsafe impl Send for Surface {}

impl Debug for Surface {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "Surface({:x})", self.id().0)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        if !self.destroyed && !thread::panicking() {
            panic!("Should have destroyed the surface first with `destroy_surface()`!")
        }
    }
}

/// Represents the CPU view of the pixel data of this surface.
pub struct SurfaceDataGuard<'a> {
    surface: &'a mut Surface,
    stride: usize,
    ptr: *mut u8,
    len: usize,
}

impl Device {
    /// Creates either a generic or a widget surface, depending on the supplied surface type.
    pub fn create_surface(
        &mut self,
        context: &Context,
        access: SurfaceAccess,
        surface_type: SurfaceType<NativeWidget>,
    ) -> Result<Surface, Error> {
        unsafe {
            let size = match surface_type {
                SurfaceType::Generic { size } => size,
                SurfaceType::Widget { .. } => panic!("Unsupported surface type for iOS: Widget")
            };
            let io_surface = self.create_io_surface(&size, access);            
            let context_id = context.id;

            Ok(Surface {
                context_id,
                io_surface,
                size,
                access,
                destroyed: false                
            })
        }
    }

    pub(crate) fn set_surface_flipped(&self, surface: &mut Surface, flipped: bool) {
        // noop
    }


    /// Destroys a surface.
    ///
    /// You must explicitly call this method to dispose of a surface. Otherwise, a panic occurs in
    /// the `drop` method.
    pub fn destroy_surface(&self, context: &Context, surface: &mut Surface) -> Result<(), Error> {
        surface.destroyed = true;
        Ok(())
    }

    /// Returns the OpenGL texture target needed to read from this surface texture.
    ///
    /// This will be `GL_TEXTURE_2D` or `GL_TEXTURE_RECTANGLE`, depending on platform.
    #[inline]
    pub fn surface_gl_texture_target(&self) -> GLenum {
        SURFACE_GL_TEXTURE_TARGET
    }

    /// Returns the OpenGL texture object containing the contents of this surface.
    ///
    /// It is only legal to read from, not write to, this texture object.
    #[inline]
    pub fn surface_texture_object(&self, surface_texture: &SurfaceTexture) -> GLuint {
        surface_texture.texture_object
    }

    /// Displays the contents of a widget surface on screen.
    ///
    /// Widget surfaces are internally double-buffered, so changes to them don't show up in their
    /// associated widgets until this method is called.
    pub fn present_surface(&self, context: &Context, surface: &mut Surface) -> Result<(), Error> {
        surface.present()
    }

    /// Resizes a widget surface
    pub fn resize_surface(&self, context: &Context, surface: &mut Surface, size: Size2D<i32>) -> Result<(), Error> {
        // noop
        Ok(())
    }

    /// Returns a pointer to the underlying surface data for reading or writing by the CPU.
    #[inline]
    pub fn lock_surface_data<'s>(
        &self,
        surface: &'s mut Surface,
    ) -> Result<SurfaceDataGuard<'s>, Error> {
        surface.lock_data()
    }

    fn create_io_surface(&self, size: &Size2D<i32>, access: SurfaceAccess) -> IOSurface {
        let cache_mode = match access {
            SurfaceAccess::GPUCPUWriteCombined => kIOMapWriteCombineCache,
            SurfaceAccess::GPUOnly | SurfaceAccess::GPUCPU => kIOMapDefaultCache,
        };

        unsafe {
            let properties = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::wrap_under_get_rule(kIOSurfaceWidth),
                    CFNumber::from(size.width).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kIOSurfaceHeight),
                    CFNumber::from(size.height).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kIOSurfaceBytesPerElement),
                    CFNumber::from(BYTES_PER_PIXEL).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kIOSurfaceBytesPerRow),
                    CFNumber::from(size.width * BYTES_PER_PIXEL).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kIOSurfacePixelFormat),
                    CFNumber::from(kCVPixelFormatType_32BGRA).as_CFType(),
                ),
                (
                    CFString::wrap_under_get_rule(kIOSurfaceCacheMode),
                    CFNumber::from(cache_mode).as_CFType(),
                ),
            ]);

            io_surface::new(&properties)
        }
    }

    /// Returns various information about the surface.
    #[inline]
    pub fn surface_info(&self, surface: &Surface) -> SurfaceInfo {
        SurfaceInfo {
            size: surface.size,
            id: surface.id(),
            context_id: surface.context_id,
            framebuffer_object: 0,            
        }
    }

    /// Returns the native `IOSurface` corresponding to this surface.
    ///
    /// The reference count is increased on the `IOSurface` before returning.
    #[inline]
    pub fn native_surface(&self, surface: &Surface) -> NativeSurface {
        let io_surface = surface.io_surface.clone();
        let io_surface_ref = io_surface.as_concrete_TypeRef();
        mem::forget(io_surface);
        NativeSurface(io_surface_ref)
    }

    pub fn create_surface_texture(
         &self,
         context: &mut Context,
         surface: Surface,
     ) -> Result<SurfaceTexture, (Error, Surface)> {
        Err((Error::UnsupportedOnThisPlatform, surface)) 
    }

    pub fn destroy_surface_texture (
         &self,
         context: &mut Context,
         surface_texture: SurfaceTexture,
     ) -> Result<Surface, (Error, SurfaceTexture)> {
        Err((Error::UnsupportedOnThisPlatform, surface_texture))
    }
}

impl Surface {
    #[inline]
    fn id(&self) -> SurfaceID {
        SurfaceID(self.io_surface.as_concrete_TypeRef() as usize)
    }

    fn present(&mut self) -> Result<(), Error> {
        Ok(())
    }

    pub(crate) fn lock_data(&mut self) -> Result<SurfaceDataGuard, Error> {
        if !self.access.cpu_access_allowed() {
            return Err(Error::SurfaceDataInaccessible);
        }

        unsafe {
            let mut seed = 0;
            let result = IOSurfaceLock(self.io_surface.as_concrete_TypeRef(), 0, &mut seed);
            if result != KERN_SUCCESS {
                return Err(Error::SurfaceLockFailed);
            }

            let ptr = IOSurfaceGetBaseAddress(self.io_surface.as_concrete_TypeRef()) as *mut u8;
            let len = IOSurfaceGetAllocSize(self.io_surface.as_concrete_TypeRef());
            let stride = IOSurfaceGetBytesPerRow(self.io_surface.as_concrete_TypeRef());

            Ok(SurfaceDataGuard {
                surface: &mut *self,
                stride,
                ptr,
                len,
            })
        }
    }
}

impl<'a> SurfaceDataGuard<'a> {
    /// Returns the number of bytes per row of the surface.
    #[inline]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Returns a mutable slice of the pixel data in this surface, in BGRA format.
    #[inline]
    pub fn data(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl<'a> Drop for SurfaceDataGuard<'a> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            let mut seed = 0;
            IOSurfaceUnlock(self.surface.io_surface.as_concrete_TypeRef(), 0, &mut seed);
        }
    }
}