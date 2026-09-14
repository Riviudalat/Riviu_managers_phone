import type { ComponentType } from "react";
import { PanelsTopLeft, Sprout, MessagesSquare, FolderOpen, PackageOpen, Workflow, ListTodo, Send, ScanLine, Database, Braces, SlidersHorizontal, Blocks, UsersRound, Router, CalendarClock, FileCheck2, CircleHelp } from "lucide-react";
import type { IconProps } from "./Icons";
/** One family, distinct silhouettes for each workspace. */
export const MENU_ICONS: Record<string, ComponentType<IconProps>> = {
  accounts: UsersRound, networks: Router, schedules: CalendarClock, savedTasks: FileCheck2, help: CircleHelp,
  control: PanelsTopLeft, myApps: Blocks, nurture: Sprout, interaction: MessagesSquare,
  material: FolderOpen, apps: PackageOpen, scripts: Workflow, jobs: ListTodo,
  publish: Send, diagnostics: ScanLine, data: Database, api: Braces, settings: SlidersHorizontal,
};
