import { Writable } from "../streams/node.mjs";

const { WritableState, fromWeb, toWeb } = Writable;
class InternalWritable extends Writable {}
Object.assign(InternalWritable, { WritableState, fromWeb, toWeb });
export { WritableState, fromWeb, toWeb };
export default InternalWritable;
