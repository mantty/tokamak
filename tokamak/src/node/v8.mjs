import { unsupported, unsupportedFunction } from "./unsupported.mjs";

export class DefaultDeserializer {}
export class DefaultSerializer {}
export class Deserializer {}
export class GCProfiler {}
export class Serializer {}
export const cachedDataVersionTag = () => 0;
export const deserialize = unsupportedFunction("v8.deserialize");
export const getCppHeapStatistics = unsupportedFunction("v8.getCppHeapStatistics");
export const getHeapCodeStatistics = unsupportedFunction("v8.getHeapCodeStatistics");
export const getHeapSnapshot = unsupportedFunction("v8.getHeapSnapshot");
export const getHeapSpaceStatistics = unsupportedFunction("v8.getHeapSpaceStatistics");
export const getHeapStatistics = () => ({ total_heap_size: 0, total_heap_size_executable: 0, total_physical_size: 0, total_available_size: 0, used_heap_size: 0, heap_size_limit: 0, malloced_memory: 0, peak_malloced_memory: 0, does_zap_garbage: 0, number_of_native_contexts: 0, number_of_detached_contexts: 0 });
export const isStringOneByteRepresentation = value => typeof value === "string" && [...value].every(character => character.charCodeAt(0) < 256);
export const promiseHooks = {};
export const queryObjects = unsupportedFunction("v8.queryObjects");
export const serialize = unsupportedFunction("v8.serialize");
export const setFlagsFromString = unsupportedFunction("v8.setFlagsFromString");
export const setHeapSnapshotNearHeapLimit = unsupportedFunction("v8.setHeapSnapshotNearHeapLimit");
export const startupSnapshot = {};
export const stopCoverage = unsupportedFunction("v8.stopCoverage");
export const takeCoverage = unsupportedFunction("v8.takeCoverage");
export const writeHeapSnapshot = unsupportedFunction("v8.writeHeapSnapshot");

export default {
  DefaultDeserializer, DefaultSerializer, Deserializer, GCProfiler, Serializer, cachedDataVersionTag, deserialize,
  getCppHeapStatistics, getHeapCodeStatistics, getHeapSnapshot, getHeapSpaceStatistics, getHeapStatistics,
  isStringOneByteRepresentation, promiseHooks, queryObjects, serialize, setFlagsFromString, setHeapSnapshotNearHeapLimit,
  startupSnapshot, stopCoverage, takeCoverage, writeHeapSnapshot,
};
