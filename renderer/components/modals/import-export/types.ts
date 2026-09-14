export type Step =
  | { name: "selectKind" }
  | { name: "selectFormat" }
  | { name: "collision"; filePath: string; itemCount: number; collisionCount: number }
  | { name: "working" }
  | { name: "done"; ok: boolean; message: string };
