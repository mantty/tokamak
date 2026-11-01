// The Intl Locale Info methods, which TypeScript's library does not declare yet.
declare namespace Intl {
  interface Locale {
    getHourCycles(): string[];
    getTextInfo(): { direction: string };
    getWeekInfo(): { firstDay: number; weekend: number[] };
  }
}
