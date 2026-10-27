//! The buffers and positions of `node:fs` reads, read from their arguments as
//! workerd reads them.

#![allow(clippy::wildcard_imports)]

use super::*;

/// The bytes `offset..offset + length` of the memory a read fills.
pub(super) struct ReadTarget<'js> {
    bytes: WritableBytes<'js>,
    offset: usize,
    length: usize,
}

/// The ranges a read fills, in order, and the position to read them from.
pub(super) struct ReadRequest<'js> {
    pub(super) targets: Vec<ReadTarget<'js>>,
    /// A position `validatePosition` accepted, `None` for `null`.
    pub(super) position: Option<Value<'js>>,
}

/// How a read reports a range that runs past the end of its buffer.
#[derive(Clone, Copy)]
pub(super) enum ReadOverrun {
    /// `readSync`'s `ERR_OUT_OF_RANGE` for the length.
    OutOfRange,
    /// The callback `fs.read`'s own `ERR_INVALID_ARG_VALUE` for the offset.
    InvalidOffset,
}

/// `fs.readSync(fd, buffer, ...args)`, as workerd's `validateReadArgs` reads
/// it: a range of the view `buffer` and the position to read it from.
pub(super) fn sync_read<'js>(
    ctx: &Ctx<'js>,
    buffer: Value<'js>,
    args: Vec<Value<'js>>,
    overrun: ReadOverrun,
) -> rquickjs::Result<ReadRequest<'js>> {
    if !is_array_buffer_view(&buffer) {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_TYPE",
            "The \"buffer\" argument must be an instance of Buffer, TypedArray, or DataView",
        ));
    }
    let view = Object::from_value(buffer)?;
    let byte_length: f64 = view.get("byteLength")?;
    let (offset, length, position) = read_fields(ctx, byte_length, args)?;
    let offset = validate_uint32(ctx, &offset, "offset")?;
    let length = validate_uint32(ctx, &length, "length")?;
    let position = validate_position(ctx, position)?;
    if length == 0 {
        return Ok(ReadRequest {
            targets: Vec::new(),
            position,
        });
    }
    let remaining = byte_length - f64::from(offset);
    if f64::from(length) > remaining {
        return Err(overrun_error(ctx, overrun, length, remaining));
    }
    let start = usize::try_from(index_property(ctx, &view, "byteOffset")? + u64::from(offset))
        .map_err(|_| out_of_bounds(ctx, "offset"))?;
    Ok(ReadRequest {
        targets: vec![buffer_target(
            ctx,
            view.get("buffer")?,
            start,
            length as usize,
        )?],
        position,
    })
}

/// A read with the arguments of Node.js's `fs.read` and `fileHandle.read`,
/// where a missing or `null` offset is 0 and a missing or `null` length runs
/// to the end of the buffer.
pub(super) fn node_read<'js>(
    ctx: &Ctx<'js>,
    arguments: ReadArguments<'js>,
    overrun: ReadOverrun,
) -> rquickjs::Result<ReadRequest<'js>> {
    let undefined = || Value::new_undefined(ctx.clone());
    let offset = defined(arguments.offset).unwrap_or_else(|| Value::new_int(ctx.clone(), 0));
    let length = defined(arguments.length).unwrap_or_else(undefined);
    let position = arguments.position.unwrap_or_else(undefined);
    sync_read(
        ctx,
        arguments.buffer,
        vec![offset, length, position],
        overrun,
    )
}

/// `fs.readvSync(fd, buffers, position)`, with `undefined` read as `null`.
pub(super) fn vector_read<'js>(
    ctx: &Ctx<'js>,
    buffers: &Value<'js>,
    position: Option<Value<'js>>,
) -> rquickjs::Result<ReadRequest<'js>> {
    let views = buffer_views(ctx, buffers)?;
    let position = optional_position(ctx, position)?;
    let targets = views
        .into_iter()
        .map(|view| view_target(ctx, view))
        .collect::<rquickjs::Result<_>>()?;
    Ok(ReadRequest { targets, position })
}

/// Fills `read`'s ranges from the file `descriptor` and counts the bytes.
pub(super) fn read_views<'js>(
    ctx: &Ctx<'js>,
    descriptor: u32,
    read: &ReadRequest<'js>,
) -> rquickjs::Result<u32> {
    if read.targets.is_empty() {
        return Ok(0);
    }
    let position = file_position(ctx, read.position.clone())?;
    let lengths: Vec<usize> = read.targets.iter().map(|target| target.length).collect();
    let chunks = vfs_call(ctx, "read", |vfs| vfs.readv(descriptor, &lengths, position))?;
    let mut total = 0;
    for (target, bytes) in read.targets.iter().zip(chunks) {
        target.bytes.write(target.offset, &bytes);
        total += bytes.len();
    }
    u32::try_from(total).map_err(|_| Exception::throw_range(ctx, "read is too large"))
}

/// The offset, length and position a read's trailing `args` give before they
/// are validated: from an options object (the default), or from a number
/// offset and the arguments after it.
fn read_fields<'js>(
    ctx: &Ctx<'js>,
    byte_length: f64,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<(Value<'js>, Value<'js>, Value<'js>)> {
    let mut args = args.into_iter();
    let first = args.next().filter(|value| !value.is_undefined());
    let null = Value::new_null(ctx.clone());
    let options = match first {
        Some(offset) if offset.is_number() => {
            let length = match defined(args.next()) {
                Some(length) => length,
                None => remaining_length(ctx, byte_length, &offset)?,
            };
            let position = args.next().filter(|value| !value.is_undefined());
            return Ok((offset, length, position.unwrap_or(null)));
        }
        Some(options) if options.is_object() && !options.is_function() => {
            Object::from_value(options)?
        }
        None => Object::new(ctx.clone())?,
        Some(_) => {
            return Err(throw_type_error(
                ctx,
                "ERR_INVALID_ARG_TYPE",
                "The \"offset\" argument must be of type number or object",
            ));
        }
    };
    let offset: Value = options.get("offset")?;
    let offset = if offset.is_undefined() {
        Value::new_int(ctx.clone(), 0)
    } else {
        offset
    };
    let length: Value = options.get("length")?;
    let length = if length.is_undefined() {
        remaining_length(ctx, byte_length, &offset)?
    } else {
        length
    };
    let position: Value = options.get("position")?;
    Ok((
        offset,
        length,
        if position.is_undefined() {
            null
        } else {
            position
        },
    ))
}

/// `byteLength - offset`, the length a read from `offset` defaults to.
fn remaining_length<'js>(
    ctx: &Ctx<'js>,
    byte_length: f64,
    offset: &Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let offset = Coerced::<f64>::from_js(ctx, offset.clone())?.0;
    Ok(Value::new_number(ctx.clone(), byte_length - offset))
}

/// The error a read of `length` bytes reports when `remaining` bytes follow
/// its offset.
fn overrun_error(
    ctx: &Ctx<'_>,
    overrun: ReadOverrun,
    length: u32,
    remaining: f64,
) -> rquickjs::Error {
    match overrun {
        ReadOverrun::OutOfRange => throw_range_error(
            ctx,
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"length\" is out of range. It must be <= {remaining}. Received {length}"
            ),
        ),
        ReadOverrun::InvalidOffset => throw_type_error(
            ctx,
            "ERR_INVALID_ARG_VALUE",
            "The argument 'offset' is invalid. Received 'must be >= 0 and <= buffer.length'",
        ),
    }
}

/// The range `view` covers in its `ArrayBuffer`.
fn view_target<'js>(ctx: &Ctx<'js>, view: Value<'js>) -> rquickjs::Result<ReadTarget<'js>> {
    let view = Object::from_value(view)?;
    let offset = usize::try_from(index_property(ctx, &view, "byteOffset")?)
        .map_err(|_| out_of_bounds(ctx, "offset"))?;
    let length = usize::try_from(index_property(ctx, &view, "byteLength")?)
        .map_err(|_| out_of_bounds(ctx, "length"))?;
    buffer_target(ctx, view.get("buffer")?, offset, length)
}

/// The bytes `offset..offset + length` of the `ArrayBuffer` `buffer`.
fn buffer_target<'js>(
    ctx: &Ctx<'js>,
    buffer: Value<'js>,
    offset: usize,
    length: usize,
) -> rquickjs::Result<ReadTarget<'js>> {
    let bytes = writable_bytes(ctx, buffer)?;
    if offset.saturating_add(length) > bytes.len {
        return Err(out_of_bounds(ctx, "length"));
    }
    Ok(ReadTarget {
        bytes,
        offset,
        length,
    })
}
