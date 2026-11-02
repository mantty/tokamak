// The Text and Data modules the test Workers import.
declare module "*.txt" {
  const text: string;
  export default text;
}

declare module "*.bin" {
  const data: ArrayBuffer;
  export default data;
}
