// surfman/surfman/src/platform/macos/cgl/context.rs
//
//! Wrapper for Core OpenGL contexts.

use super::device::Device;
use crate::info::GLVersion;
use crate::context::{ContextID, CREATE_CONTEXT_MUTEX};
use crate::{ContextAttributes, Error, Gl, SurfaceInfo};
use glutin::dpi::PhysicalSize;
use glutin::event_loop::EventLoop;
use glutin::{
    Context as GlutinContext, ContextBuilder, ContextCurrentState, CreationError, GlProfile, GlRequest, NotCurrent,
};

pub struct Context {
    pub(crate) id: ContextID,
    pub(crate) glutin_ctx: GlutinContext<NotCurrent>
}

pub struct ContextDescriptor {
    pub(crate) gl_version: GLVersion
}

pub struct NativeContext();

impl Device {
    pub fn create_context(
        &mut self,
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
            glutin_ctx: gl_ctx,
        };

        next_context_id.0 += 1;
        Ok(ctx)
    }
}