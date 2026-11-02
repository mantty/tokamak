/** An error with Node's `code` property. */
export type CodedError = Error & { code: string };

export function codedError(ErrorType: new (message: string) => Error, message: string, code: string): CodedError {
  const error = new ErrorType(message);
  return Object.assign(error, { code });
}

// Node's AbortError, with `cause` as its cause when one is given.
export function abortError(cause?: unknown): CodedError {
  const error = new Error("The operation was aborted", cause === undefined ? undefined : { cause });
  const coded = Object.assign(error, { code: "ABORT_ERR" });
  coded.name = "AbortError";
  return coded;
}
