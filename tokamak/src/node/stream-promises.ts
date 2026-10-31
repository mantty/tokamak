// Shares streams/node.js's promises object, so stream.promises and stream/promises are the same object.
import { promises } from "../streams/node.js";

export const { finished, pipeline } = promises;
export default promises;
