// Fails a test file that would not exit once its tests finish, naming what it left
// open, instead of letting the run wait for it.
import { after } from "node:test";

after(() => {
  setTimeout(() => {
    console.error(`${process.argv[1]} left resources open after its tests: ${process.getActiveResourcesInfo().join(", ")}`);
    process.exit(1);
  }, 10_000).unref();
});
