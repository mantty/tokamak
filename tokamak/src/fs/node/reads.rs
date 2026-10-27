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

/// The buffer a read fills and the range and position it asks for, before
/// `readSync` checks them.
pub(super) struct ReadFields<'js> {
    pub(super) buffer: Value<'js>,
    offset: Value<'js>,
    length: Value<'js>,
    position: Value<'js>,
}

impl<'js> ReadFields<'js> {
    /// All `size` bytes of `buffer`, read from the descriptor's own position.
    fn whole(ctx: &Ctx<'js>, buffer: Value<'js>, size: f64) -> Self {
        Self {
            buffer,
            offset: Value::new_int(ctx.clone(), 0),
            length: Value::new_number(ctx.clone(), size),
            position: Value::new_null(ctx.clone()),
        }
    }

    /// The ranges `readSync(fd, buffer, offset, length, position)` fills, after
    /// its checks of each field.
    pub(super) fn request(&self, ctx: &Ctx<'js>) -> rquickjs::Result<ReadRequest<'js>> {
        let byte_length = view_byte_length(ctx, &self.buffer)?;
        let offset = validate_uint32(ctx, &self.offset, "offset")?;
        let length = validate_uint32(ctx, &self.length, "length")?;
        let position = validate_position(ctx, self.position.clone(), "position")?;
        if length == 0 {
            return Ok(ReadRequest {
                targets: Vec::new(),
                position,
            });
        }
        let remaining = byte_length - f64::from(offset);
        if f64::from(length) > remaining {
            return Err(throw_range_error(
                ctx,
                "ERR_OUT_OF_RANGE",
                &format!(
                    "The value of \"length\" is out of range. It must be <= {remaining}. Received {length}"
                ),
            ));
        }
        let target: Value = read_buffer(ctx)?.call((self.buffer.clone(), offset, length))?;
        Ok(ReadRequest {
            targets: vec![view_target(ctx, &target)?],
            position,
        })
    }

    /// Whether the read asks for no bytes, which the callback `fs.read`
    /// answers without reading.
    pub(super) fn is_empty(&self, ctx: &Ctx<'js>) -> rquickjs::Result<bool> {
        Ok(self.length.as_number() == Some(0.0) || view_byte_length(ctx, &self.buffer)? == 0.0)
    }

    /// workerd's checks of the offset, length and position, named with
    /// `prefix`.
    fn validate(&self, ctx: &Ctx<'js>, prefix: &str) -> rquickjs::Result<()> {
        validate_uint32(ctx, &self.offset, &format!("{prefix}offset"))?;
        validate_uint32(ctx, &self.length, &format!("{prefix}length"))?;
        validate_position(ctx, self.position.clone(), &format!("{prefix}position"))?;
        Ok(())
    }
}

/// `fs.readSync(fd, buffer, ...args)`, as workerd's `validateReadArgs` reads
/// it: a range of the view `buffer` and the position to read it from.
pub(super) fn sync_read<'js>(
    ctx: &Ctx<'js>,
    buffer: Value<'js>,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<ReadRequest<'js>> {
    let byte_length = view_byte_length(ctx, &buffer)?;
    let (offset, length, position) = read_fields(ctx, byte_length, args)?;
    ReadFields {
        buffer,
        offset,
        length,
        position,
    }
    .request(ctx)
}

/// The read `fs.read(fd, ...args)` asks for and its callback, as workerd reads
/// `args`: `(callback)`, `(options, callback)` or a buffer and the arguments
/// `buffer_read` reads.
pub(super) fn callback_read<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<(ReadFields<'js>, Function<'js>)> {
    let first = argument(ctx, args, 0);
    let (fields, callback) = if first.is_function() {
        let buffer = new_read_buffer(ctx)?;
        let size = view_byte_length(ctx, &buffer)?;
        (ReadFields::whole(ctx, buffer, size), first)
    } else if first.is_object() && !is_array_buffer_view(&first) {
        let fields = options_fields(ctx, &first, None)?;
        if !is_array_buffer_view(&fields.buffer) {
            return Err(invalid_buffer(ctx, "options.buffer"));
        }
        fields.validate(ctx, "options.")?;
        (fields, argument(ctx, args, 1))
    } else {
        buffer_read(ctx, first, args.get(1..).unwrap_or_default())?
    };
    let callback = Function::from_value(callback).map_err(|_| {
        throw_type_error(
            ctx,
            "ERR_INVALID_ARG_TYPE",
            "The \"callback\" argument must be of type function",
        )
    })?;
    let size = view_byte_length(ctx, &fields.buffer)?;
    if outside_buffer(ctx, &fields.offset, &fields.length, size)? {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_VALUE",
            "The argument 'offset' is invalid. Received 'must be >= 0 and <= buffer.length'",
        ));
    }
    Ok((fields, callback))
}

/// The read a callback `fs.read` into `buffer` asks for and its callback, as
/// workerd reads the arguments after the buffer: `(callback)`,
/// `(options, callback)`, `(offset, callback)` or
/// `(offset, length, position, callback)`.
fn buffer_read<'js>(
    ctx: &Ctx<'js>,
    buffer: Value<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<(ReadFields<'js>, Value<'js>)> {
    let size = view_byte_length(ctx, &buffer)?;
    let first = argument(ctx, args, 0);
    if first.is_function() {
        return Ok((ReadFields::whole(ctx, buffer, size), first));
    }
    if (first.is_object() || first.is_null()) && !is_number_object(ctx, &first)? {
        let fields = options_fields(ctx, &first, Some(buffer))?;
        fields.validate(ctx, "options.")?;
        return Ok((fields, argument(ctx, args, 1)));
    }
    let offset = if first.is_number() {
        first
    } else {
        Value::new_int(ctx.clone(), 0)
    };
    let second = argument(ctx, args, 1);
    let (length, position, callback) = if second.is_function() {
        let length = remaining_length(ctx, size, &offset)?;
        (length, Value::new_null(ctx.clone()), second)
    } else {
        let length = match defined(Some(second)) {
            Some(length) => length,
            None => remaining_length(ctx, size, &offset)?,
        };
        let position = argument(ctx, args, 2);
        validate_uint32(ctx, &position, "position")?;
        (length, position, argument(ctx, args, 3))
    };
    let fields = ReadFields {
        buffer,
        offset,
        length,
        position,
    };
    Ok((fields, callback))
}

/// The read `fileHandle.read(...args)` asks for, as workerd reads `args`:
/// `(buffer, offset, length?, position?)`, `(buffer, options?)` or
/// `(options?)`.
pub(super) fn file_handle_read<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<ReadFields<'js>> {
    let (first, second) = (argument(ctx, args, 0), argument(ctx, args, 1));
    let fields = if is_array_buffer_view(&first) && second.is_number() {
        let length = match defined(Some(argument(ctx, args, 2))) {
            Some(length) => length,
            None => remaining_length(ctx, view_byte_length(ctx, &first)?, &second)?,
        };
        let position =
            defined(Some(argument(ctx, args, 3))).unwrap_or_else(|| Value::new_null(ctx.clone()));
        ReadFields {
            buffer: first,
            offset: second,
            length,
            position,
        }
    } else {
        let options = if is_array_buffer_view(&first) {
            with_buffer(ctx, first, &second)?
        } else if first.is_undefined() {
            Object::new(ctx.clone())?.into_value()
        } else {
            first
        };
        options_fields(ctx, &options, None)?
    };
    fields.validate(ctx, "")?;
    Ok(fields)
}

/// The argument at `index`, `undefined` when there is none.
fn argument<'js>(ctx: &Ctx<'js>, args: &[Value<'js>], index: usize) -> Value<'js> {
    args.get(index)
        .cloned()
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()))
}

/// The fields of the read options `options`, as workerd destructures them:
/// `{ buffer = Buffer.alloc(16384), offset = 0, length = buffer.byteLength -
/// offset, position = null }`, with `buffer` taken from the arguments instead
/// when the read names one.
fn options_fields<'js>(
    ctx: &Ctx<'js>,
    options: &Value<'js>,
    buffer: Option<Value<'js>>,
) -> rquickjs::Result<ReadFields<'js>> {
    let buffer = match buffer {
        Some(buffer) => buffer,
        None => match property(ctx, options, "buffer")? {
            buffer if buffer.is_undefined() => new_read_buffer(ctx)?,
            buffer => buffer,
        },
    };
    let offset = match property(ctx, options, "offset")? {
        offset if offset.is_undefined() => Value::new_int(ctx.clone(), 0),
        offset => offset,
    };
    let length = match property(ctx, options, "length")? {
        length if length.is_undefined() => {
            let size = Coerced::<f64>::from_js(ctx, property(ctx, &buffer, "byteLength")?)?.0;
            remaining_length(ctx, size, &offset)?
        }
        length => length,
    };
    let position = match property(ctx, options, "position")? {
        position if position.is_undefined() => Value::new_null(ctx.clone()),
        position => position,
    };
    Ok(ReadFields {
        buffer,
        offset,
        length,
        position,
    })
}

/// `{ buffer, ...options }`.
fn with_buffer<'js>(
    ctx: &Ctx<'js>,
    buffer: Value<'js>,
    options: &Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let merged = Object::new(ctx.clone())?;
    merged.set("buffer", buffer)?;
    if let Some(options) = options.as_object() {
        for key in options.keys::<rquickjs::Atom>() {
            let key = key?;
            merged.set(key.clone(), options.get::<_, Value>(key)?)?;
        }
    }
    Ok(merged.into_value())
}

/// `value[name]`, which throws for `null` and `undefined`. Of the primitives,
/// only a string has a property a read asks for: its `length`.
fn property<'js>(ctx: &Ctx<'js>, value: &Value<'js>, name: &str) -> rquickjs::Result<Value<'js>> {
    if let Some(object) = value.as_object() {
        return object.get(name);
    }
    if value.is_null() || value.is_undefined() {
        let kind = if value.is_null() { "null" } else { "undefined" };
        return Err(Exception::throw_type(
            ctx,
            &format!("cannot read property '{name}' of {kind}"),
        ));
    }
    match value.as_string() {
        Some(string) if name == "length" => string.to_string()?.encode_utf16().count().into_js(ctx),
        _ => Ok(Value::new_undefined(ctx.clone())),
    }
}

/// Whether `value instanceof Number`.
fn is_number_object<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> rquickjs::Result<bool> {
    let number: Value = ctx.globals().get("Number")?;
    Ok(value
        .as_object()
        .is_some_and(|object| object.is_instance_of(&number)))
}

/// Whether the callback `fs.read` range `offset`, `length` falls outside a
/// buffer of `size` bytes, by workerd's check. The length can still be any
/// value the caller passed, so the check keeps JavaScript's comparison and
/// addition.
fn outside_buffer<'js>(
    ctx: &Ctx<'js>,
    offset: &Value<'js>,
    length: &Value<'js>,
    size: f64,
) -> rquickjs::Result<bool> {
    let check: Function = cached(ctx, "__tokamak_node_fs_outside_buffer", |ctx| {
        ctx.eval(
            "(offset, length, size) => offset < 0 || length < 0 || (length !== 0 && offset + length > size)",
        )
    })?;
    check.call((offset.clone(), length.clone(), size))
}

/// `Buffer.alloc(16384)`, the buffer a read fills when given none.
fn new_read_buffer<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    let class = buffer_class(ctx)?;
    let alloc: Function = class.get("alloc")?;
    alloc.call((This(class), 16_384))
}

/// The `byteLength` of `buffer`, which must be a `Buffer`, `TypedArray` or
/// `DataView`.
fn view_byte_length<'js>(ctx: &Ctx<'js>, buffer: &Value<'js>) -> rquickjs::Result<f64> {
    if !is_array_buffer_view(buffer) {
        return Err(invalid_buffer(ctx, "buffer"));
    }
    Object::from_value(buffer.clone())?.get("byteLength")
}

fn invalid_buffer(ctx: &Ctx<'_>, name: &str) -> rquickjs::Error {
    throw_type_error(
        ctx,
        "ERR_INVALID_ARG_TYPE",
        &format!(
            "The \"{name}\" {} must be an instance of Buffer, TypedArray, or DataView",
            argument_kind(name)
        ),
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
        .map(|view| view_target(ctx, &view))
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

/// workerd's `readSync` target,
/// `Buffer.from(buffer.buffer, buffer.byteOffset + offset, length)`, which
/// reads the properties a Worker may define on the view.
fn read_buffer<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    cached(ctx, "__tokamak_node_fs_read_buffer", |ctx| {
        let buffer: Value =
            crate::runtime_modules::module_namespace(ctx.clone(), "tokamak:node/buffer.mjs")?
                .get("Buffer")?;
        let target: Function = ctx.eval(
            "Buffer => (view, offset, length) => Buffer.from(view.buffer, view.byteOffset + offset, length)",
        )?;
        target.call((buffer,))
    })
}

/// The range `view` covers in its `ArrayBuffer`, as the engine holds it:
/// workerd's native reads ignore properties a Worker defines on the view.
fn view_target<'js>(ctx: &Ctx<'js>, view: &Value<'js>) -> rquickjs::Result<ReadTarget<'js>> {
    let parts = crate::globals::view_parts(ctx, view)?;
    buffer_target(ctx, parts.buffer, parts.offset, parts.length)
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
