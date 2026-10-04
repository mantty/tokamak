// Text and Data modules, compared with workerd by modules-reference.mjs.
import text from "./text.txt";
import * as textNamespace from "./text.txt";
import bom from "./bom.txt";
import invalid from "./invalid.txt";
import data from "./data.bin";
import * as dataNamespace from "./data.bin";

const codePoints = (value) => Array.from(value, (character) => character.codePointAt(0));

export default {
  async fetch() {
    const again = await import("./data.bin");
    return Response.json({
      text,
      bom: codePoints(bom),
      invalid: codePoints(invalid),
      textKeys: Object.keys(textNamespace),
      dataKeys: Object.keys(dataNamespace),
      tag: Object.prototype.toString.call(data),
      bytes: Array.from(new Uint8Array(data)),
      sameData: again.default === data,
      resizable: data.resizable,
    });
  },
};
