export function unsupported(name) {
  throw Object.assign(new Error(`${name} is not available in the Tokamak runtime`), { code: "ERR_METHOD_NOT_IMPLEMENTED" });
}

export function unsupportedFunction(name) {
  return () => unsupported(name);
}
