import { Writable, WritableState, fromWeb, toWeb } from "../streams/node.mjs";

class InternalWritable extends Writable {}
Object.assign(InternalWritable, { WritableState, fromWeb, toWeb });
export { WritableState, fromWeb, toWeb };
export default InternalWritable;
