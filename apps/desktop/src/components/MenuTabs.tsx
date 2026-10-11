import type { PageId } from "../types";
import { menuItemFor } from "./navigationMenu";

/**
 * The tab row of a sidebar entry that holds more than one page ("Ứng dụng": Quy trình ứng dụng ·
 * Gói cài đặt). Each tab is its own page with its own state; the row only moves between them.
 */
export function MenuTabs({ page, onPage }: { page: PageId; onPage: (page: PageId) => void }) {
  const item = menuItemFor(page);
  if (!item?.tabs) return null;
  return (
    <nav className="automation-tabs menu-tabs" aria-label={`Trang ${item.label}`}>
      {item.tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          aria-current={tab.id === page ? "page" : undefined}
          onClick={() => { if (tab.id !== page) onPage(tab.id); }}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
