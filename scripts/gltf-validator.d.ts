/** Types for the parts of the Khronos glTF validator that scripts/assets.ts uses. */
declare module 'gltf-validator' {
  interface Message {
    code: string;
    message: string;
    /** 0 error, 1 warning, 2 info, 3 hint */
    severity: number;
    pointer?: string;
  }
  interface Report {
    issues: { numErrors: number; numWarnings: number; messages: Message[] };
  }
  export function validateBytes(data: Uint8Array, options?: { maxIssues?: number }): Promise<Report>;
  const validator: { validateBytes: typeof validateBytes };
  export default validator;
}
