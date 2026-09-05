import { describe, expect, it } from "vitest";

import publishCssRaw from "./styles/publish.css?raw";

const withoutComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, " ");

function declarations(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = withoutComments(publishCssRaw).match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  if (!match) throw new Error(`Missing CSS rule for ${selector}`);
  return match[1];
}

describe("publish workspace scroll ownership", () => {
  it("keeps the page as a bounded column and gives vertical scrolling to its active panel", () => {
    const page = declarations(".publish-page");
    expect(page).toMatch(/display:\s*flex\s*;/);
    expect(page).toMatch(/flex-direction:\s*column\s*;/);
    expect(page).toMatch(/min-height:\s*0\s*;/);
    expect(page).toMatch(/overflow:\s*hidden\s*;/);

    const section = declarations(".publish-workspace-section");
    expect(section).toMatch(/flex:\s*1\s+1\s+0\s*;/);
    expect(section).toMatch(/min-height:\s*0\s*;/);
    expect(section).toMatch(/overflow:\s*auto\s*;/);
  });
});
