import { Readable } from "../streams/node.mjs";

const { ReadableState, _fromList, from, fromWeb, toWeb, wrap } = Readable;
class InternalReadable extends Readable {}
Object.assign(InternalReadable, { ReadableState, _fromList, from, fromWeb, toWeb, wrap });
export { ReadableState, _fromList, from, fromWeb, toWeb, wrap };
export default InternalReadable;
