import { describe, expect, it } from "vitest";
import { canonicalSiteUrl, parseSiteRoute, sitePath } from "./siteRoutes";
import { postLoginDestination } from "./routeAccess";

describe("Domain-based project routes", () => {
  const site = { primary_domain: "testing.incogmsg.my.id" };
  it("uses the primary domain for entry and nested tabs", () => {
    expect(sitePath(site)).toBe("/sites/testing.incogmsg.my.id/overview");
    expect(sitePath(site, "ssl")).toBe("/sites/testing.incogmsg.my.id/ssl");
    expect(sitePath({ primary_domain: "EXAMPLE.COM" }, "files")).toBe(
      "/sites/example.com/files",
    );
  });
  it.each(["/sites/testing.incogmsg.my.id", "/sites/testing.incogmsg.my.id/"])(
    "opens overview from %s",
    (path) => {
      expect(parseSiteRoute(path)).toEqual({
        key: site.primary_domain,
        suffix: "/overview",
      });
    },
  );
  it("keeps bookmarked tabs, query strings and fragments on legacy URLs", () => {
    expect(
      canonicalSiteUrl(
        site,
        "/sites/1/settings/runtime",
        "?tab=php",
        "#limits",
      ),
    ).toBe("/sites/testing.incogmsg.my.id/settings/runtime?tab=php#limits");
    expect(canonicalSiteUrl(site, "/sites/1")).toBe(sitePath(site));
    expect(postLoginDestination(sitePath(site, "ssl"), "user")).toBe(
      sitePath(site, "ssl"),
    );
  });
  it("recognizes numeric bookmarks and decodes internationalized domains", () => {
    expect(parseSiteRoute("/sites/42/ssl")).toEqual({
      key: "42",
      suffix: "/ssl",
    });
    expect(parseSiteRoute(sitePath({ primary_domain: "toko-é.test" }))).toEqual(
      { key: "toko-é.test", suffix: "/overview" },
    );
  });
  it.each([
    "/sites",
    "/sites/",
    "/sites/%",
    "/sites/a%2Fb/ssl",
    "/sites/a%5Cb/ssl",
    "/sites/a%00b",
    "/sites/..",
  ])("rejects malformed routes %s", (path) => {
    expect(parseSiteRoute(path)).toBeNull();
  });
});
