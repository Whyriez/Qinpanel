import { describe, expect, it, vi } from "vitest";
import { renderToString } from "react-dom/server";
import { Shell } from "../Shell";
import { AuthProvider } from "../../../context/AuthContext";
import "../../../i18n";

describe("Hosting console shell", () => {
  it('uses a readable breadcrumb for an unknown page', () => {
    const html = renderToString(<AuthProvider>
      <Shell theme="light" onThemeChange={vi.fn()} activeNav="missing-page"><p>404</p></Shell>
    </AuthProvider>);
    expect(html).toContain('Halaman tidak ditemukan.');
    expect(html).not.toContain('nav.missing-page');
  });

  it("renders accessible navigation, search and theme controls without placeholder server identity", () => {
    const html = renderToString(
      <AuthProvider>
        <Shell theme="light" onThemeChange={vi.fn()} activeNav="sites">
          <p>Workspace content</p>
        </Shell>
      </AuthProvider>,
    );
    expect(html).toContain("Workspace content");
    expect(html).toContain('aria-current="page"');
    expect(html).toContain("Lewati ke konten");
    expect(html).toContain("Ganti tema");
    expect(html).toContain("Buka navigasi");
    expect(html).not.toMatch(/vps-01|alpha|Rust/);
    expect(html).not.toContain('0 aktif');
  });
});
