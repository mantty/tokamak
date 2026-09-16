export function unsupported(name) {
  const error = new Error(`${name} is not available in the Tokamak runtime`);
  error.code = "ERR_METHOD_NOT_IMPLEMENTED";
  throw error;
}

export function unsupportedFunction(name) {
  return (..._args) => unsupported(name);
}
