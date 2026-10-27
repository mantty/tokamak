//! The data and positions of `node:fs` writes, read from their arguments as
//! workerd reads them.

#![allow(clippy::wildcard_imports)]

use super::*;

/// The largest integer a JavaScript number holds exactly.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// A write's data, the bytes of each view it writes and the position to write
/// them at.
pub(super) struct WriteRequest<'js> {
    /// The data value the write reports back.
    pub(super) data: Value<'js>,
    pub(super) buffers: Vec<Vec<u8>>,
    /// A position `validatePosition` accepted, `None` for `null`.
    pub(super) position: Option<Value<'js>>,
}

/// `fs.writeSync(fd, value, ...args)`.
pub(super) fn sync_write<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<WriteRequest<'js>> {
    if !value.is_string() {
        return view_write(ctx, value, args);
    }
    let mut args = args.into_iter();
    let position = optional_position(ctx, args.next())?;
    let encoding = args
        .next()
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    validate_hex_length(ctx, &value, &encoding)?;
    let buffer = encoded(ctx, value, encoding)?;
    Ok(WriteRequest {
        buffers: vec![view_bytes(ctx, &buffer)?],
        data: buffer,
        position,
    })
}

/// `fs.write(fd, value, ...args, callback)`: a string is encoded with a string
/// second argument, which the write then ignores.
pub(super) fn callback_write<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    mut args: Vec<Value<'js>>,
) -> rquickjs::Result<WriteRequest<'js>> {
    if !value.is_string() {
        return view_write(ctx, value, args);
    }
    let encoding = match args.get_mut(1) {
        Some(encoding) if encoding.is_string() => {
            std::mem::replace(encoding, Value::new_undefined(ctx.clone()))
        }
        _ => "utf8".into_js(ctx)?,
    };
    view_write(ctx, encoded(ctx, value, encoding)?, args)
}

/// `fileHandle.write(value, ...args)`: a string is encoded with the second
/// argument, which the write still reads as its length.
pub(super) fn file_handle_write<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<WriteRequest<'js>> {
    if !value.is_string() {
        return view_write(ctx, value, args);
    }
    let encoding = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    view_write(ctx, encoded(ctx, value, encoding)?, args)
}

/// `fs.writevSync(fd, buffers, position)`, with `None` for a `null` position.
pub(super) fn vector_write<'js>(
    ctx: &Ctx<'js>,
    buffers: Value<'js>,
    position: Option<Value<'js>>,
) -> rquickjs::Result<WriteRequest<'js>> {
    let views = buffer_views(ctx, &buffers)?;
    let position = position.map_or(Ok(None), |position| validate_position(ctx, position))?;
    Ok(WriteRequest {
        buffers: views
            .iter()
            .map(|view| view_bytes(ctx, view))
            .collect::<rquickjs::Result<_>>()?,
        data: buffers,
        position,
    })
}

/// The views in `buffers`, as `validateBufferArray` accepts them.
pub(super) fn buffer_views<'js>(
    ctx: &Ctx<'js>,
    buffers: &Value<'js>,
) -> rquickjs::Result<Vec<Value<'js>>> {
    let views: Vec<Value> = buffers
        .as_array()
        .ok_or_else(|| invalid_buffers(ctx))?
        .iter()
        .collect::<rquickjs::Result<_>>()?;
    if !views.iter().all(is_array_buffer_view) {
        return Err(invalid_buffers(ctx));
    }
    Ok(views)
}

/// The file offset `position` names, converted as workerd's native write
/// converts it.
pub(super) fn file_position<'js>(
    ctx: &Ctx<'js>,
    position: Option<Value<'js>>,
) -> rquickjs::Result<Option<u64>> {
    let Some(position) = position else {
        return Ok(None);
    };
    if !position.is_big_int() {
        return Coerced::<u64>::from_js(ctx, position).map(|position| Some(position.0));
    }
    Coerced::<String>::from_js(ctx, position)?
        .0
        .parse()
        .map(Some)
        .map_err(|_| {
            Exception::throw_type(
                ctx,
                "The value cannot be converted because it is either negative and this API expects a positive bigint, or the value would be truncated.",
            )
        })
}

/// A write of `length` bytes from `offset` in the view `value`, with the
/// offset, length and position its trailing `args` or options object give.
fn view_write<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<WriteRequest<'js>> {
    if !is_array_buffer_view(&value) {
        return Err(invalid_bytes(ctx, "buffer"));
    }
    let view = Object::from_value(value.clone())?;
    let byte_length = index_property(ctx, &view, "byteLength")?;
    let (offset, length, position) = write_fields(args)?;
    let offset = offset.map_or(Ok(0), |offset| validate_integer(ctx, offset, "offset"))?
        + index_property(ctx, &view, "byteOffset")?;
    let position = position.map_or(Ok(None), |position| validate_position(ctx, position))?;
    let length = length.map_or(Ok(byte_length), |length| {
        validate_integer(ctx, length, "length")
    })?;
    if length > byte_length {
        return Err(out_of_bounds(ctx, "length"));
    }
    if offset > length {
        return Err(out_of_bounds(ctx, "offset"));
    }
    Ok(WriteRequest {
        buffers: vec![buffer_range(ctx, &view.get("buffer")?, offset, length)?],
        data: value,
        position,
    })
}

/// The offset, length and position a write's trailing `args` give, each
/// `None` when it takes its default.
fn write_fields(
    args: Vec<Value<'_>>,
) -> rquickjs::Result<(Option<Value<'_>>, Option<Value<'_>>, Option<Value<'_>>)> {
    let mut args = args.into_iter();
    let first = args.next();
    let Some(options) = first
        .as_ref()
        .filter(|value| value.is_object() && !value.is_function())
        .and_then(Value::as_object)
    else {
        return Ok((defined(first), defined(args.next()), defined(args.next())));
    };
    let length: Value = options.get("length")?;
    Ok((
        defined(options.get("offset")?),
        (!length.is_undefined()).then_some(length),
        defined(options.get("position")?),
    ))
}

/// The view a whole-file write of `data` writes once `options` pass
/// workerd's checks: `data` itself, or the string `data` encoded as `options`
/// name.
pub(super) fn file_view<'js>(
    ctx: &Ctx<'js>,
    data: Value<'js>,
    options: Option<&Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    check_encoding(ctx, options, EncodingCheck::NullRejected)?;
    if is_array_buffer_view(&data) {
        return Ok(data);
    }
    if !data.is_string() {
        return Err(invalid_bytes(ctx, "data"));
    }
    let encoding = parse_options(ctx, options.cloned())?.encoding;
    encoded(ctx, data, encoding.into_js(ctx)?)
}

/// The bytes `WriteStream.write` writes for `chunk`: a view's, or a string's
/// encoded in `encoding`.
pub(super) fn chunk_bytes<'js>(
    ctx: &Ctx<'js>,
    chunk: Value<'js>,
    encoding: Value<'js>,
) -> rquickjs::Result<Vec<u8>> {
    if chunk.is_string() {
        return view_bytes(ctx, &encoded(ctx, chunk, encoding)?);
    }
    if !is_array_buffer_view(&chunk) {
        return Err(invalid_bytes(ctx, "chunk"));
    }
    view_bytes(ctx, &chunk)
}

/// The bytes `view` covers, with none when its buffer is detached.
pub(super) fn attached_bytes<'js>(ctx: &Ctx<'js>, view: &Value<'js>) -> rquickjs::Result<Vec<u8>> {
    // A detached view's length is 0.
    if index_property(ctx, &Object::from_value(view.clone())?, "byteLength")? == 0 {
        return Ok(Vec::new());
    }
    view_bytes(ctx, view)
}

/// The bytes `view` covers.
fn view_bytes<'js>(ctx: &Ctx<'js>, view: &Value<'js>) -> rquickjs::Result<Vec<u8>> {
    let view = Object::from_value(view.clone())?;
    let offset = index_property(ctx, &view, "byteOffset")?;
    let length = index_property(ctx, &view, "byteLength")?;
    buffer_range(ctx, &view.get("buffer")?, offset, length)
}

/// `length` bytes from `offset` in the `ArrayBuffer` `buffer`.
fn buffer_range<'js>(
    ctx: &Ctx<'js>,
    buffer: &Value<'js>,
    offset: u64,
    length: u64,
) -> rquickjs::Result<Vec<u8>> {
    let buffer_object = Object::from_value(buffer.clone())?;
    let available = index_property(ctx, &buffer_object, "byteLength")?
        .checked_sub(offset)
        .ok_or_else(|| out_of_bounds(ctx, "offset"))?;
    if length > available {
        return Err(out_of_bounds(ctx, "length"));
    }
    let bytes = ArrayBuffer::from_value(buffer.clone())
        .and_then(|buffer| buffer.as_bytes().map(<[u8]>::to_vec))
        .ok_or(rquickjs::Error::Exception)?;
    let start = usize::try_from(offset).map_err(|_| out_of_bounds(ctx, "offset"))?;
    let end = usize::try_from(offset + length).map_err(|_| out_of_bounds(ctx, "length"))?;
    Ok(bytes[start..end].to_vec())
}

/// `Buffer.from(text, encoding)`.
fn encoded<'js>(
    ctx: &Ctx<'js>,
    text: Value<'js>,
    encoding: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let class = buffer_class(ctx)?;
    let from: Function = class.get("from")?;
    from.call((This(class), text, encoding))
}

/// Rejects hex text of odd length, as `validateEncoding` does.
fn validate_hex_length<'js>(
    ctx: &Ctx<'js>,
    text: &Value<'js>,
    encoding: &Value<'js>,
) -> rquickjs::Result<()> {
    let hex = encoding
        .as_string()
        .map(rquickjs::String::to_string)
        .transpose()?
        .is_some_and(|encoding| encoding.eq_ignore_ascii_case("hex"));
    let text = Coerced::<String>::from_js(ctx, text.clone())?.0;
    if hex && text.encode_utf16().count() % 2 != 0 {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_VALUE",
            "The argument 'encoding' is invalid for data of odd length",
        ));
    }
    Ok(())
}

/// A typed array or `DataView`.
pub(super) fn is_array_buffer_view(value: &Value<'_>) -> bool {
    // SAFETY: Both calls only read the class of a live value.
    unsafe { qjs::JS_GetTypedArrayType(value.as_raw()) >= 0 || qjs::JS_IsDataView(value.as_raw()) }
}

/// The non-negative integer property `name` of `object`.
pub(super) fn index_property<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    name: &str,
) -> rquickjs::Result<u64> {
    Coerced::<u64>::from_js(ctx, object.get(name)?).map(|value| value.0)
}

/// `value` as `validateInteger(value, name, 0)` accepts it.
fn validate_integer<'js>(ctx: &Ctx<'js>, value: Value<'js>, name: &str) -> rquickjs::Result<u64> {
    let Some(number) = value.as_number() else {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_TYPE",
            &format!("The \"{name}\" argument must be of type number"),
        ));
    };
    if !is_integer(number) {
        return Err(out_of_range(ctx, name, "an integer"));
    }
    if !(0.0..=MAX_SAFE_INTEGER).contains(&number) {
        return Err(out_of_range(ctx, name, ">= 0 && <= 9007199254740991"));
    }
    Coerced::<u64>::from_js(ctx, value).map(|value| value.0)
}

/// `value` validated as a position, with `undefined` read as `null`.
pub(super) fn optional_position<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<Option<Value<'js>>> {
    defined(value).map_or(Ok(None), |value| validate_position(ctx, value))
}

/// `value` as `validatePosition` accepts it: `null`, a bigint or an unsigned
/// 32-bit integer.
pub(super) fn validate_position<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<Option<Value<'js>>> {
    if value.is_null() {
        return Ok(None);
    }
    if value.is_big_int() {
        return Ok(Some(value));
    }
    if !value.is_number() {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_TYPE",
            "The \"position\" argument must be of type bigint or one of integer or null",
        ));
    }
    validate_uint32(ctx, &value, "position")?;
    Ok(Some(value))
}

/// `value` as `validateUint32(value, name)` accepts it.
pub(super) fn validate_uint32<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    name: &str,
) -> rquickjs::Result<u32> {
    validate_integer_range(
        ctx,
        value,
        name,
        0.0..=f64::from(u32::MAX),
        ">= 0 && < 4294967296",
    )
}

/// The file descriptor `value` names, as `getValidatedFd` accepts it.
pub(super) fn descriptor_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<u32> {
    let range = 0.0..=f64::from(i32::MAX);
    validate_integer_range(ctx, &value, "fd", range, ">= 0 && <= 2147483647")
}

/// `value` when it is an integer number in `range`, which `text` describes.
fn validate_integer_range<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    name: &str,
    range: std::ops::RangeInclusive<f64>,
    text: &str,
) -> rquickjs::Result<u32> {
    let Some(number) = value.as_number() else {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_TYPE",
            &format!("The \"{name}\" argument must be of type number"),
        ));
    };
    if !is_integer(number) {
        return Err(out_of_range(ctx, name, "an integer"));
    }
    if !range.contains(&number) {
        return Err(out_of_range(ctx, name, text));
    }
    let integer = Coerced::<i64>::from_js(ctx, value.clone())?.0;
    u32::try_from(integer).map_err(|_| out_of_range(ctx, name, text))
}

fn is_integer(number: f64) -> bool {
    number.is_finite() && number.fract() == 0.0
}

fn out_of_range(ctx: &Ctx<'_>, name: &str, range: &str) -> rquickjs::Error {
    throw_range_error(
        ctx,
        "ERR_OUT_OF_RANGE",
        &format!("The value of \"{name}\" is out of range. It must be {range}"),
    )
}

fn invalid_bytes(ctx: &Ctx<'_>, name: &str) -> rquickjs::Error {
    throw_type_error(
        ctx,
        "ERR_INVALID_ARG_TYPE",
        &format!(
            "The \"{name}\" argument must be of type string or an instance of Buffer, TypedArray, or DataView"
        ),
    )
}

fn invalid_buffers(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_type_error(
        ctx,
        "ERR_INVALID_ARG_TYPE",
        "The \"buffer\" argument must be an instance of ArrayBufferView[]",
    )
}

pub(super) fn out_of_bounds(ctx: &Ctx<'_>, name: &str) -> rquickjs::Error {
    throw_range_error(
        ctx,
        "ERR_BUFFER_OUT_OF_BOUNDS",
        &format!("\"{name}\" is outside of buffer bounds"),
    )
}
