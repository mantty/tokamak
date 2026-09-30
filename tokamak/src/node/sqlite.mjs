import { unsupported, unsupportedFunction } from "./unsupported.mjs";

export class DatabaseSync {
  constructor() { unsupported("sqlite.DatabaseSync"); }
}
export class StatementSync {}
export const backup = unsupportedFunction("sqlite.backup");
export const constants = {};
export default { DatabaseSync, StatementSync, backup, constants };
