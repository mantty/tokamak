import { flag, nonEmptyText, object, oneOf } from "./checks.js";

/** Throws `TypeError` unless `options` are options `requestPermission` takes. */
export const validatePermissionRequest = object({
  background: flag,
  precision: oneOf("precise", "approximate"),
  purpose: nonEmptyText,
});
