import { Duplex } from "../streams/node.mjs";

const { from, fromWeb, toWeb } = Duplex;
class InternalDuplex extends Duplex {}
Object.assign(InternalDuplex, { from, fromWeb, toWeb });
export { from, fromWeb, toWeb };
export default InternalDuplex;
