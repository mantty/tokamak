/** The operations a web page performs itself, used by the generated client in a web deployment. */
export const web = {
  /** The page's URL. */
  getCurrentPage(): Promise<string | null> {
    return Promise.resolve(globalThis.location?.href ?? null);
  },
};
