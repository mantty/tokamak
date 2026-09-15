import { installWebGlobals } from "../globals/web.mjs";
import { installWebSocketGlobals } from "../network/websocket.mjs";

installWebGlobals();
installWebSocketGlobals();
globalThis.global = globalThis;
delete globalThis.InternalError;
