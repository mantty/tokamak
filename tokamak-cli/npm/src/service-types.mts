export interface ServiceMethod {
  name: string;
  overloads: number;
  documentation: string;
}

/** Retain the original types while making every exposed overload asynchronous. */
export function serviceTypes(instance: string, methods: ServiceMethod[]): string {
  const counts = [...new Set(methods.map((method) => method.overloads))].sort((a, b) => a - b);
  const helpers = counts.map((count) => {
    const signatures = Array.from({ length: count }, (_, index) => index);
    const match = signatures.map((index) => `(...args: infer A${index}): infer R${index};`);
    const wrapped = signatures.map((index) => `(...args: A${index}): Promise<Awaited<R${index}>>;`);
    return `type Async${count}<T> = T extends {\n  ${match.join("\n  ")}\n} ? {\n  ${wrapped.join("\n  ")}\n} : never;`;
  });
  const members = methods.map(({ name, overloads, documentation }) => {
    const comment = documentation ? `  /** ${documentation.replaceAll("*/", "* /").replaceAll("\n", "\n   * ")} */\n` : "";
    return `${comment}  ${JSON.stringify(name)}: Async${overloads}<Worker[${JSON.stringify(name)}]>;`;
  });
  return `type Worker = ${instance};

${helpers.join("\n\n")}

export interface TokamakClient {
${members.join("\n")}
}

export interface TokamakService extends TokamakClient {
  fetch(input: Request | string | URL, init?: RequestInit): Promise<Response>;
}
`;
}
