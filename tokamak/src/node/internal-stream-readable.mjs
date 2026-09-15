import { Readable, ReadableState, _fromList, from, fromWeb, toWeb } from "../streams/node.mjs";

function wrap(value) { return value; }
class InternalReadable extends Readable {}
Object.assign(InternalReadable, { ReadableState, _fromList, from, fromWeb, toWeb, wrap });
export { ReadableState, _fromList, from, fromWeb, toWeb, wrap };
export default InternalReadable;
