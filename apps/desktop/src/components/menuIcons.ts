import type { ReactElement } from "react";

import type { IconProps } from "./Icons";
import {
  IconApi,
  IconApp,
  IconChart,
  IconClock,
  IconGrid,
  IconImage,
  IconRocket,
  IconScript,
  IconSettings,
} from "./Icons";

/**
 * Icon for each sidebar destination.
 *
 * Not in `Icons.tsx`: a lookup table is not a component, and exporting one from a file of
 * components is what costs that file its Fast Refresh.
 */
export const MENU_ICONS: Record<string, (p: IconProps) => ReactElement> = {
  control: IconGrid,
  material: IconImage,
  apps: IconApp,
  scripts: IconScript,
  jobs: IconClock,
  publish: IconRocket,
  data: IconChart,
  api: IconApi,
  settings: IconSettings,
};
