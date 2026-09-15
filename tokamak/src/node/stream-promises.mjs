// node:stream/promises re-exports the promises API vendored into
// streams/node.mjs so that stream.promises and stream/promises are the same
// objects, as in Node.
import { promises } from "../streams/node.mjs";

export const { finished, pipeline } = promises;
export default promises;
