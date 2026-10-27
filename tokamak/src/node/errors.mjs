export function codedError(ErrorType, message, code) {
  const error = new ErrorType(message);
  error.code = code;
  return error;
}
