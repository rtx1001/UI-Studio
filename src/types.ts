export type AssetInfo = {
  path: string;
  relativePath: string;
  directory: string;
  filename: string;
  extension: "png" | "jpg" | "jpeg";
  width: number;
  height: number;
  hasAlpha: boolean;
  bytes: number;
};

export type InventoryIssue = { path: string; kind: string; message: string };
export type InventoryReport = {
  root: string;
  assets: AssetInfo[];
  issues: InventoryIssue[];
  collisions: string[][];
  totalFilesSeen: number;
};

export type ProcessResult = {
  relativePath: string;
  outputPath: string;
  status: "complete" | "failed" | "skipped";
  valid: boolean;
  message: string;
  sourceWidth: number;
  sourceHeight: number;
  outputWidth: number | null;
  outputHeight: number | null;
  alphaPreserved: boolean;
};

export type QueueStatus = "queued" | "processing" | "complete" | "failed" | "skipped";

export type PaletteColor = { hex: string; weight: number };

export type Settings = {
  sourceRoot: string;
  outputRoot: string;
  stylePrompt: string;
  paletteColors: PaletteColor[];
  loraEnabled: boolean;
  loraStrength: number;
  fluxDenoisingStrength: number;
  structurePreservation: number;
  inferenceSteps: 4 | 8 | 12;
  overwrite: boolean;
  view: "grid" | "list";
  groupBy: "none" | "name" | "size" | "dimensions";
  styleLockEnabled: boolean;
  styleSeed: number;
  workingResolution: 256 | 512 | 768 | 1024 | 2048;
  minimumResolutionEnabled: boolean;
  contentAwareScaling: boolean;
  leftPanelWidth: number;
  rightPanelWidth: number;
};
