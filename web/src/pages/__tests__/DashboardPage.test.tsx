import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import { DashboardPage } from "../DashboardPage";
import { AuthProvider } from "../../context/AuthContext";
import "../../i18n";

describe("Hosting dashboard", () => {
  it("renders an honest loading state without example sites or manufactured status", () => {
    const html = renderToString(
      <AuthProvider>
        <DashboardPage />
      </AuthProvider>,
    );
    expect(html).toContain("Ringkasan hosting");
    expect(html).not.toContain("Memeriksa koneksi");
    expect(html).toContain("Memuat informasi hosting");
    expect(html).not.toMatch(
      /example\.com|acme\.dev|18\.4%|Phase|axum|RPC|Rust/,
    );
    expect(html).not.toContain("Server terhubung");
    expect(html).toContain("dashboard-sites-title");
  });
});
