import { Duplex, from, fromWeb, toWeb } from "../streams/node.mjs";

class InternalDuplex extends Duplex {}
Object.assign(InternalDuplex, { from, fromWeb, toWeb });
export { from, fromWeb, toWeb };
export default InternalDuplex;
