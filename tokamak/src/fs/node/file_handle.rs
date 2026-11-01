#![allow(clippy::wildcard_imports)]

//! The host state of `node:fs/promises`'s `FileHandle`, the class that
//! `fs/node/file-handle.ts` defines over it.

use std::cell::Cell;

use rquickjs::Class;
use rquickjs::JsLifetime;
use rquickjs::class::Trace;
use tokamak_macros::JsType;

use super::*;
use crate::typed::JsReadableStream;

/// An open descriptor, which a `FileHandle`'s methods use until `close`.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct FileHandleState {
    #[qjs(skip_trace)]
    descriptor: Cell<Option<u32>>,
}

/// What `read` resolves to.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct ReadResult<'js> {
    bytes_read: usize,
    buffer: Value<'js>,
}

/// What `readv` resolves to.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct ReadvResult<'js> {
    bytes_read: usize,
    buffers: Value<'js>,
}

/// What `write` and `writeFile` resolve to.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct WriteResult<'js> {
    bytes_written: usize,
    buffer: Value<'js>,
}

/// What `writev` resolves to.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct WritevResult<'js> {
    bytes_written: usize,
    buffers: Value<'js>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods(rename_all = "camelCase")]
impl FileHandleState {
    /// The descriptor, until the handle is closed.
    #[qjs(get)]
    pub fn fd(&self) -> Option<u32> {
        self.descriptor.get()
    }

    pub fn read<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<ReadResult<'js>> {
        let descriptor = self.open(&ctx)?;
        let read = file_handle_read(&ctx, &args.0)?;
        let count = read_views(&ctx, descriptor, &read.request(&ctx)?)?;
        Ok(ReadResult {
            bytes_read: count as usize,
            buffer: read.buffer,
        })
    }

    pub fn readv<'js>(
        &self,
        ctx: Ctx<'js>,
        buffers: Value<'js>,
        position: Opt<Value<'js>>,
    ) -> rquickjs::Result<ReadvResult<'js>> {
        let descriptor = self.open(&ctx)?;
        let count = read_views(&ctx, descriptor, &vector_read(&ctx, &buffers, position.0)?)?;
        Ok(ReadvResult {
            bytes_read: count as usize,
            buffers,
        })
    }

    pub fn read_file<'js>(
        &self,
        ctx: Ctx<'js>,
        options: Opt<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        let descriptor = self.open(&ctx)?;
        let options = parse_options(&ctx, options.0)?;
        let bytes = read_descriptor(&ctx, descriptor)?;
        output(ctx, &bytes, options.encoding.as_deref())
    }

    pub fn write<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<WriteResult<'js>> {
        let descriptor = self.open(&ctx)?;
        let mut args = args.0.into_iter();
        let value = args
            .next()
            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let write = write_request(&ctx, value, args.collect(), file_handle_string_write)?;
        let count = write_views(&ctx, descriptor, &write)?;
        Ok(WriteResult {
            bytes_written: count as usize,
            buffer: write.data,
        })
    }

    pub fn writev<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<WritevResult<'js>> {
        let descriptor = self.open(&ctx)?;
        let mut args = args.0.into_iter();
        let buffers = args
            .next()
            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let write = vector_write(&ctx, buffers, defined(args.next()))?;
        let count = write_views(&ctx, descriptor, &write)?;
        Ok(WritevResult {
            bytes_written: count as usize,
            buffers: write.data,
        })
    }

    pub fn write_file<'js>(
        &self,
        ctx: Ctx<'js>,
        data: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> rquickjs::Result<WriteResult<'js>> {
        let descriptor = self.open(&ctx)?;
        let count = write_all(
            &ctx,
            descriptor.into_js(&ctx)?,
            data.clone(),
            options.0.clone(),
        )?;
        // workerd reports a string as `Buffer.from(data, options)` encodes it.
        let buffer = if is_array_buffer_view(&data) {
            data
        } else {
            let options = options
                .0
                .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            encoded(&ctx, data, options)?
        };
        Ok(WriteResult {
            bytes_written: count,
            buffer,
        })
    }

    pub fn append_file<'js>(
        &self,
        ctx: Ctx<'js>,
        data: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> rquickjs::Result<()> {
        let descriptor = self.open(&ctx)?;
        append_file_sync(ctx.clone(), descriptor.into_js(&ctx)?, data, options).map(drop)
    }

    pub fn stat<'js>(
        &self,
        ctx: Ctx<'js>,
        options: Opt<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let descriptor = self.open(&ctx)?;
        fstat_sync(ctx, descriptor, options)
    }

    pub fn truncate(&self, ctx: Ctx<'_>, length: Opt<Option<u64>>) -> rquickjs::Result<()> {
        let descriptor = self.open(&ctx)?;
        ftruncate_sync(ctx, descriptor, Opt(length.0.flatten()))
    }

    pub fn sync(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        let descriptor = self.open(&ctx)?;
        fsync_sync(ctx, descriptor)
    }

    pub fn chmod<'js>(&self, ctx: Ctx<'js>, mode: Value<'js>) -> rquickjs::Result<()> {
        let descriptor = self.open(&ctx)?;
        fchmod_sync(ctx, descriptor, mode)
    }

    pub fn chown(&self, ctx: Ctx<'_>, uid: u32, gid: u32) -> rquickjs::Result<()> {
        let descriptor = self.open(&ctx)?;
        fchown_sync(ctx, descriptor, uid, gid)
    }

    pub fn utimes<'js>(
        &self,
        ctx: Ctx<'js>,
        atime: Value<'js>,
        mtime: Value<'js>,
    ) -> rquickjs::Result<()> {
        let descriptor = self.open(&ctx)?;
        futimes_sync(ctx, descriptor, atime, mtime)
    }

    /// Closes the descriptor; a closed handle stays closed.
    pub fn close(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.descriptor
            .take()
            .map_or(Ok(()), |descriptor| close_descriptor(&ctx, descriptor))
    }

    pub fn readable_web_stream<'js>(
        &self,
        ctx: Ctx<'js>,
    ) -> rquickjs::Result<JsReadableStream<'js>> {
        let stream = readable_web_stream(&ctx, self.open(&ctx)?)?;
        JsReadableStream::from_js(&ctx, stream.into_value())
    }
}

impl FileHandleState {
    /// The handle's descriptor, or once it is closed the error workerd's
    /// methods reject with.
    fn open(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u32> {
        self.descriptor.get().ok_or_else(|| closed_error(ctx))
    }
}

/// workerd's `EBADF` error for a method of a closed `FileHandle`.
fn closed_error(ctx: &Ctx<'_>) -> rquickjs::Error {
    let thrown = crate::errors::new_error(ctx, "Bad file descriptor").and_then(|exception| {
        let object = exception.as_object();
        object.set("code", "EBADF")?;
        object.set("name", "Error")?;
        object.set("syscall", "stat")?;
        object.set("errno", -9)?;
        Ok(exception.throw())
    });
    thrown.unwrap_or_else(|error| error)
}

/// `fs/node/file-handle.ts`, which defines `FileHandle`.
pub(super) fn file_handle_module<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    cached(ctx, "__tokamak_node_fs_file_handle", |ctx| {
        crate::runtime_modules::module_namespace(ctx.clone(), "tokamak:fs/node/file-handle.ts")
    })
}

/// A `FileHandle` for the open `descriptor`.
pub(super) fn file_handle<'js>(ctx: &Ctx<'js>, descriptor: u32) -> rquickjs::Result<Value<'js>> {
    let state = Class::instance(
        ctx.clone(),
        FileHandleState {
            descriptor: Cell::new(Some(descriptor)),
        },
    )?;
    let create: Function = file_handle_module(ctx)?.get("fileHandle")?;
    create.call((state,))
}

/// A `ReadableStream` of the rest of the file `descriptor`.
fn readable_web_stream<'js>(ctx: &Ctx<'js>, descriptor: u32) -> rquickjs::Result<Object<'js>> {
    let bytes = read_descriptor(ctx, descriptor)?;
    let constructor: Constructor = ctx
        .globals()
        .get("ReadableStream")
        .map_err(|_| Exception::throw_internal(ctx, "ReadableStream is not installed"))?;
    let source = Object::new(ctx.clone())?;
    source.set(
        "start",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, controller: Object<'js>| {
                let enqueue: Function = controller.get("enqueue")?;
                enqueue.call::<_, ()>((TypedArray::<u8>::new_copy(ctx.clone(), &bytes)?,))?;
                let close: Function = controller.get("close")?;
                close.call::<_, ()>(())?;
                Ok::<(), rquickjs::Error>(())
            },
        )?,
    )?;
    constructor.construct((source,))
}
