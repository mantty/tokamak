// The name and code of a thrown error.
export function failure(error) {
  return { error: error.name, code: error.code ?? null };
}

// What `callback` returns, or the failure it throws.
export function outcome(callback) {
  try { return callback(); }
  catch (error) { return failure(error); }
}

// What `callback` resolves to, or the failure it throws or rejects with.
export async function asyncOutcome(callback) {
  try { return await callback(); }
  catch (error) { return failure(error); }
}
