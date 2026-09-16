import { unsupportedFunction } from "./unsupported.mjs";

export const SCHED_NONE = 1;
export const SCHED_RR = 2;
export class Worker {}
export const _events = Object.create(null);
export const _eventsCount = 0;
export const _maxListeners = undefined;
export const domain = undefined;
export const isMaster = false;
export const isPrimary = false;
export const isWorker = false;
export const schedulingPolicy = SCHED_RR;
export const settings = {};
export const workers = {};

export default {
  SCHED_NONE, SCHED_RR, Worker, _events, _eventsCount, _maxListeners, domain,
  isMaster, isPrimary, isWorker, schedulingPolicy, settings, workers,
};
