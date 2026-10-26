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

/// `fs.readSync(fd, buffer, ...args)`, as workerd's `validateReadArgs` reads
/// it. The range may run past the view to the end of its `ArrayBuffer`.
pub(super) fn sync_read<'js>(
    ctx: &Ctx<'js>,
    buffer: Value<'js>,
    args: Vec<Value<'js>>,
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
    let offset = match added_number(&offset) {
        Some(offset) => Value::new_number(ctx.clone(), view.get::<_, f64>("byteOffset")? + offset),
        None => offset,
    };
    let offset = validate_uint32(ctx, &offset, "offset")? as usize;
    let length = validate_uint32(ctx, &length, "length")? as usize;
    let position = validate_position(ctx, position)?;
    let bytes = writable_bytes(ctx, view.get("buffer")?)?;
    if offset.saturating_add(length) > bytes.len {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_VALUE",
            &format!("The argument 'offset' out of bounds. Received {offset}"),
        ));
    }
    let empty = length == 0 || byte_length <= 0.0;
    Ok(ReadRequest {
        targets: if empty {
            Vec::new()
        } else {
            vec![ReadTarget {
                bytes,
                offset,
                length,
            }]
        },
        position,
    })
}

/// A read with the arguments of Node.js's `fs.read` and `fileHandle.read`,
/// where a missing or `null` offset is 0 and a missing or `null` length runs
/// to the end of the buffer.
pub(super) fn node_read<'js>(
    ctx: &Ctx<'js>,
    arguments: ReadArguments<'js>,
) -> rquickjs::Result<ReadRequest<'js>> {
    let undefined = || Value::new_undefined(ctx.clone());
    let offset = defined(arguments.offset).unwrap_or_else(|| Value::new_int(ctx.clone(), 0));
    let length = defined(arguments.length).unwrap_or_else(undefined);
    let position = arguments.position.unwrap_or_else(undefined);
    sync_read(ctx, arguments.buffer, vec![offset, length, position])
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
    let chunks = vfs_call(ctx, |vfs| vfs.readv(descriptor, &lengths, position))?;
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
    let rest = |offset: &Value<'js>| {
        let offset = added_number(offset).unwrap_or_default();
        Value::new_number(ctx.clone(), byte_length - offset)
    };
    let null = Value::new_null(ctx.clone());
    let options = match first {
        Some(offset) if offset.is_number() => {
            let length = defined(args.next()).unwrap_or_else(|| rest(&offset));
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
    let length: Value = options.get("length")?;
    let position: Value = options.get("position")?;
    Ok((
        offset.clone(),
        if length.is_undefined() {
            rest(&offset)
        } else {
            length
        },
        if position.is_undefined() {
            null
        } else {
            position
        },
    ))
}

/// The number `value` adds to a number in JavaScript, or `None` when the
/// addition makes a string.
fn added_number(value: &Value<'_>) -> Option<f64> {
    if value.is_undefined() || value.is_null() {
        return Some(0.0);
    }
    value
        .as_bool()
        .map(|value| f64::from(u8::from(value)))
        .or_else(|| value.as_number())
}

/// The range `view` covers in its `ArrayBuffer`.
fn view_target<'js>(ctx: &Ctx<'js>, view: Value<'js>) -> rquickjs::Result<ReadTarget<'js>> {
    let view = Object::from_value(view)?;
    let offset = usize::try_from(index_property(ctx, &view, "byteOffset")?)
        .map_err(|_| out_of_bounds(ctx, "offset"))?;
    let length = usize::try_from(index_property(ctx, &view, "byteLength")?)
        .map_err(|_| out_of_bounds(ctx, "length"))?;
    let bytes = writable_bytes(ctx, view.get("buffer")?)?;
    if offset.saturating_add(length) > bytes.len {
        return Err(out_of_bounds(ctx, "length"));
    }
    Ok(ReadTarget {
        bytes,
        offset,
        length,
    })
}
