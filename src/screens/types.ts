import type { LocalFlowSettings } from "../domain/types";

/** Change the settings; the change is saved shortly afterwards. */
export type UpdateSettings = (updater: (settings: LocalFlowSettings) => LocalFlowSettings) => void;
