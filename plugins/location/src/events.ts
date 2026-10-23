import type { Listener } from "@tokamakdev/plugin/events";

import * as android from "./android.generated.js";
import * as apple from "./apple.generated.js";
import type { LocationErrorEvent, LocationUpdate, RegionTransition, Visit } from "./types.js";

/**
 * Receives the positions of continuous updates and significant changes in native builds: on
 * screen, off screen, or after the system launches the app for them.
 */
export function onLocationUpdate(listener: Listener<LocationUpdate>): void {
  apple.onUpdate(listener);
  android.onUpdate(listener);
}

/** Receives each region entry or exit, including one that launches the app. */
export function onRegionTransition(listener: Listener<RegionTransition>): void {
  apple.onRegion(listener);
  android.onRegion(listener);
}

/** Apple only: receives each arrival at or departure from a place, including one that launches the app. */
export function onVisit(listener: Listener<Visit>): void {
  apple.onVisit(listener);
}

/** Receives failures of background location services, such as a region the system cannot monitor. */
export function onLocationError(listener: Listener<LocationErrorEvent>): void {
  apple.onError(listener);
  android.onError(listener);
}
