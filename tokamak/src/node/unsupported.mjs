import { codedError } from "./errors.mjs";

export function unsupported(name) {
  throw codedError(Error, `${name} is not available in the Tokamak runtime`, "ERR_METHOD_NOT_IMPLEMENTED");
}

export function unsupportedFunction(name) {
  return () => unsupported(name);
}
