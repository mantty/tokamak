import { codedError } from "./errors.ts";

export function unsupported(name: string): never {
  throw codedError(Error, `${name} is not available in the Tokamak runtime`, "ERR_METHOD_NOT_IMPLEMENTED");
}

export function unsupportedFunction(name: string): () => never {
  return () => unsupported(name);
}
