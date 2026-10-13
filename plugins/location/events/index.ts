import { defineEvent } from "@tokamakdev/plugin/events";

import type { LocationErrorEvent, LocationUpdate, RegionTransition, Visit } from "../src/index.js";

/**
 * Receives the positions of continuous updates and significant changes in native builds: on
 * screen, off screen, or after the system launches the app for them.
 */
export const onLocationUpdate = defineEvent<LocationUpdate>("location.update");

/** Receives each region entry or exit, including one that launches the app. */
export const onRegionTransition = defineEvent<RegionTransition>("location.region");

/** Apple only: receives each arrival at or departure from a place, including one that launches the app. */
export const onVisit = defineEvent<Visit>("location.visit");

/** Receives failures of background location services, such as a region the system cannot monitor. */
export const onLocationError = defineEvent<LocationErrorEvent>("location.error");
