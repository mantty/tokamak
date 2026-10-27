export function codedError(ErrorType, message, code) {
  const error = new ErrorType(message);
  error.code = code;
  return error;
}

// Node's AbortError, with `cause` as its cause when one is given.
export function abortError(cause) {
  const error = new Error("The operation was aborted", cause === undefined ? undefined : { cause });
  error.code = "ABORT_ERR";
  error.name = "AbortError";
  return error;
}
