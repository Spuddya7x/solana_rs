/**
 * Navigation: named places, a router, and an executor.
 *
 * ```ts
 * import { travelTo, NAV, capabilities, here } from './lib/nav';
 *
 * await travelTo(bot, sdk, 'aubury');            // go buy runes
 * const bank = NAV.nearestTagged(here(sdk), 'bank', capabilities(sdk));
 * ```
 */

export * from './types';
export * from './places';
export * from './links';
export * from './graph';
export * from './travel';
