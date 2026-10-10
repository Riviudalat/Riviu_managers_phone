import { Autocomplete } from "@base-ui/react/autocomplete";
import { Dialog } from "@base-ui/react/dialog";
import { Search } from "lucide-react";
import { useId, useRef } from "react";
import "./commandPalette.css";

/**
 * The app's command palette: Base UI's Dialog around an inline Autocomplete, drawn with plain
 * CSS through the same scoped aliases as `Menu.tsx`.
 *
 * Base UI owns the parts a hand-rolled palette gets wrong: the `combobox`/`listbox`/`option`
 * roles, the highlight that follows ArrowUp/ArrowDown while focus stays in the search box,
 * Enter on the highlighted row, Escape, and focus going back to whatever was focused before
 * the palette opened. Filtering is ours, below, because Base UI's default does not ignore
 * Vietnamese diacritics and "may 3" has to find "Máy 3".
 *
 * **A row only navigates or selects.** The chosen row runs after the dialog has finished
 * closing, so the dialog hands focus back first and a row that moves focus (to a tile, say)
 * gets the last word. A row that would run work on a phone must open that work's own
 * confirmation instead of running it.
 */

export interface PaletteCommand {
  id: string;
  label: string;
  /** Short trailing text: what Enter does, or why the row is disabled. */
  hint?: string;
  /** Extra words the filter matches that the label does not show (a handle, a page alias). */
  keywords?: string;
  disabled?: boolean;
  run: () => void;
}

export interface PaletteGroup {
  label: string;
  items: PaletteCommand[];
}

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The dialog's accessible name. */
  label: string;
  placeholder: string;
  groups: PaletteGroup[];
}

function normalizePaletteText(value: string): string {
  return value
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[đĐ]/g, "d")
    .toLocaleLowerCase("vi")
    .replace(/\s+/g, " ")
    .trim();
}

/** Every word of the query must appear, in any order, in the label or the keywords. */
function matches(item: PaletteCommand, query: string): boolean {
  const words = normalizePaletteText(query).split(" ").filter(Boolean);
  if (!words.length) return true;
  const haystack = normalizePaletteText(`${item.label} ${item.keywords ?? ""}`);
  return words.every((word) => haystack.includes(word));
}

export function CommandPalette({ open, onOpenChange, label, placeholder, groups }: CommandPaletteProps) {
  const hintId = useId();
  const chosen = useRef<PaletteCommand | null>(null);
  const shown = groups.filter((group) => group.items.length > 0);

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next) => onOpenChange(next)}
      onOpenChangeComplete={(next) => {
        if (next) return;
        const command = chosen.current;
        chosen.current = null;
        command?.run();
      }}
    >
      <Dialog.Portal>
        <Dialog.Backdrop className="ui-palette-backdrop" />
        <Dialog.Viewport className="ui-palette-viewport">
          <Dialog.Popup className="ui-palette" aria-label={label}>
            <Autocomplete.Root
              open
              inline
              items={shown}
              filter={matches}
              itemToStringValue={(item: PaletteCommand) => item.label}
              autoHighlight="always"
              keepHighlight
            >
              <div className="ui-palette-search">
                <Search size={16} aria-hidden="true" />
                <Autocomplete.Input
                  className="ui-palette-input"
                  aria-label={label}
                  aria-describedby={hintId}
                  placeholder={placeholder}
                />
              </div>
              <Dialog.Close className="visually-hidden">Đóng {label.toLocaleLowerCase("vi")}</Dialog.Close>
              <Autocomplete.Empty className="ui-palette-empty">Không có mục nào khớp.</Autocomplete.Empty>
              <Autocomplete.List className="ui-palette-list" aria-label={label}>
                {(group: PaletteGroup) => (
                  <Autocomplete.Group key={group.label} items={group.items} className="ui-palette-group">
                    <Autocomplete.GroupLabel className="ui-palette-group-label">{group.label}</Autocomplete.GroupLabel>
                    <Autocomplete.Collection>
                      {(item: PaletteCommand) => (
                        <Autocomplete.Item
                          key={item.id}
                          value={item}
                          disabled={item.disabled}
                          className="ui-palette-item"
                          onClick={() => {
                            if (item.disabled) return;
                            chosen.current = item;
                            onOpenChange(false);
                          }}
                        >
                          <span className="ui-palette-text">{item.label}</span>
                          {item.hint && <span className="ui-palette-hint">{item.hint}</span>}
                        </Autocomplete.Item>
                      )}
                    </Autocomplete.Collection>
                  </Autocomplete.Group>
                )}
              </Autocomplete.List>
              <p id={hintId} className="ui-palette-footer">
                <kbd>↑</kbd><kbd>↓</kbd> chọn dòng · <kbd>Enter</kbd> thực hiện · <kbd>Esc</kbd> đóng
              </p>
            </Autocomplete.Root>
          </Dialog.Popup>
        </Dialog.Viewport>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
