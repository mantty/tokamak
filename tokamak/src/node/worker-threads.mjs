import { MessageChannel, MessagePort } from "../events/web.mjs";
import { unsupportedFunction } from "./unsupported.mjs";

export { MessageChannel, MessagePort };
export class BroadcastChannel {}
export const SHARE_ENV = Symbol("worker_threads.SHARE_ENV");
export class Worker {
  constructor() { unsupportedFunction("worker_threads.Worker")(); }
}
export const getEnvironmentData = () => undefined;
export const isInternalThread = false;
export const isMainThread = true;
export const isMarkedAsUntransferable = () => false;
export const markAsUncloneable = () => {};
export const markAsUntransferable = () => {};
export const moveMessagePortToContext = unsupportedFunction("worker_threads.moveMessagePortToContext");
export const parentPort = null;
export const postMessageToThread = unsupportedFunction("worker_threads.postMessageToThread");
export const receiveMessageOnPort = unsupportedFunction("worker_threads.receiveMessageOnPort");
export const resourceLimits = {};
export const setEnvironmentData = () => {};
export const threadId = 0;
export const workerData = undefined;
export default {
  BroadcastChannel, MessageChannel, MessagePort, SHARE_ENV, Worker, getEnvironmentData,
  isInternalThread, isMainThread, isMarkedAsUntransferable, markAsUncloneable, markAsUntransferable,
  moveMessagePortToContext, parentPort, postMessageToThread, receiveMessageOnPort, resourceLimits,
  setEnvironmentData, threadId, workerData,
};
