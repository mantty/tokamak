/** The operations a web page performs itself, used by the generated client in a web deployment. */
export const web = {
  /** Returns the message without native code. */
  echo(message: string): Promise<string> {
    return Promise.resolve(message);
  },
};
