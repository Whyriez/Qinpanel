import { describe, it, expect } from "vitest";
import { renderToString } from "react-dom/server";
import { PulseStrip } from "../PulseStrip";

describe("PulseStrip Component", () => {
  it("renders SVG sparkline path with supplied data", () => {
    const html = renderToString(
      <PulseStrip data={[10, 20, 15, 30, 25]} width={100} height={20} />,
    );
    expect(html).toContain("<svg");
    expect(html).toContain('role="img"');
    expect(html).toContain("Aktivitas real-time sparkline");
    expect(html).toContain("<path");
  });

  it("renders danger color when hasError is true", () => {
    const html = renderToString(
      <PulseStrip data={[5, 10, 8]} hasError={true} />,
    );
    expect(html).toContain("var(--danger, #ef4444)");
  });

  it("renders primary color when hasError is false", () => {
    const html = renderToString(
      <PulseStrip data={[10, 20, 30]} hasError={false} />,
    );
    expect(html).toContain("var(--primary, #3b82f6)");
  });

  it("marks the latest point without continuous decorative animation", () => {
    const html = renderToString(<PulseStrip data={[10, 25, 40]} />);
    expect(html).toContain("<circle");
    expect(html).not.toContain("animate-ping");
  });

  it("can scale to its container with an accurate accessible label", () => {
    const html = renderToString(
      <PulseStrip
        data={[12, 19, 15]}
        width={400}
        height={112}
        responsive
        label="Penggunaan CPU"
      />,
    );
    expect(html).toContain('viewBox="0 0 400 112"');
    expect(html).toContain('width="100%"');
    expect(html).toContain('aria-label="Penggunaan CPU"');
  });
});
