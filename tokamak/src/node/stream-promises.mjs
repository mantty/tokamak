// Shares streams/node.mjs's promises object, so stream.promises and stream/promises are the same object.
import { promises } from "../streams/node.mjs";

export const { finished, pipeline } = promises;
export default promises;
