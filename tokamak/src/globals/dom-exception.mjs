import { registerDomException } from "tokamak:host";
import { markHostObject } from "./objects.mjs";

const legacyCodes = {
  IndexSizeError: 1, DOMStringSizeError: 2, HierarchyRequestError: 3, WrongDocumentError: 4,
  InvalidCharacterError: 5, NoModificationAllowedError: 7, NotFoundError: 8,
  NotSupportedError: 9, InUseAttributeError: 10, InvalidStateError: 11,
  SyntaxError: 12, InvalidModificationError: 13, NamespaceError: 14,
  InvalidAccessError: 15, TypeMismatchError: 17, SecurityError: 18,
  NetworkError: 19, AbortError: 20, URLMismatchError: 21, QuotaExceededError: 22,
  TimeoutError: 23, DataCloneError: 25, NotAllowedError: 0,
};

export class DOMException extends Error {
  constructor(message = "", name = "Error") {
    super(String(message));
    markHostObject(this, "DOMException");
    this.name = String(name);
    this.code = Object.hasOwn(legacyCodes, this.name) ? legacyCodes[this.name] : 0;
  }
  get [Symbol.toStringTag]() { return "DOMException"; }
}

for (const [name, code] of Object.entries({
  INDEX_SIZE_ERR: 1, DOMSTRING_SIZE_ERR: 2, HIERARCHY_REQUEST_ERR: 3, WRONG_DOCUMENT_ERR: 4,
  INVALID_CHARACTER_ERR: 5, NO_DATA_ALLOWED_ERR: 6, NO_MODIFICATION_ALLOWED_ERR: 7,
  NOT_FOUND_ERR: 8, NOT_SUPPORTED_ERR: 9, INUSE_ATTRIBUTE_ERR: 10, INVALID_STATE_ERR: 11,
  SYNTAX_ERR: 12, INVALID_MODIFICATION_ERR: 13, NAMESPACE_ERR: 14, INVALID_ACCESS_ERR: 15,
  TYPE_MISMATCH_ERR: 17, SECURITY_ERR: 18, NETWORK_ERR: 19, ABORT_ERR: 20,
  URL_MISMATCH_ERR: 21, QUOTA_EXCEEDED_ERR: 22, TIMEOUT_ERR: 23, INVALID_NODE_TYPE_ERR: 24,
  DATA_CLONE_ERR: 25, VALIDATION_ERR: 0,
})) {
  DOMException[name] = code;
  DOMException.prototype[name] = code;
}

registerDomException(DOMException);
