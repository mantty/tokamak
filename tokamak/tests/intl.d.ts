// Intl methods that TypeScript's library does not declare yet.
declare namespace Intl {
  interface Locale {
    getHourCycles(): string[];
    getTextInfo(): { direction: string };
    getWeekInfo(): { firstDay: number; weekend: number[] };
  }

  // The plural category of a range, from ECMA-402 2023.
  interface PluralRules {
    selectRange(start: number, end: number): LDMLPluralRule;
  }
}
