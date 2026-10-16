import module from "./compute.wasm";

const instance = await WebAssembly.instantiate(module);

export default {
  fetch() {
    return Response.json({ answer: instance.exports.add(20, 22) });
  },
};
