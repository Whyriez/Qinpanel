import { useEffect, useState } from "react";
import { fetchSites } from "../api/client";
import { SiteDetailPage } from "./SiteDetailPage";

/** Resolve domains only from the signed-in account's authorized site list. */
export function SiteDetailRoute({
  siteKey,
  onBack,
}: {
  siteKey: string;
  onBack: () => void;
}) {
  const numericId = /^\d+$/.test(siteKey) ? Number(siteKey) : null;
  const [resolved, setResolved] = useState<{
    key: string;
    id?: number;
    error?: string;
  } | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (numericId !== null) return;
    let cancelled = false;
    fetchSites()
      .then((sites) => {
        if (cancelled) return;
        const site = sites.find(
          (item) => item.primary_domain.toLowerCase() === siteKey.toLowerCase(),
        );
        setResolved(
          site
            ? { key: siteKey, id: site.id }
            : {
                key: siteKey,
                error: "Situs tidak ditemukan atau Anda tidak memiliki akses.",
              },
        );
      })
      .catch((error: unknown) => {
        if (!cancelled)
          setResolved({
            key: siteKey,
            error:
              error instanceof Error ? error.message : "Gagal memuat situs.",
          });
      });
    return () => {
      cancelled = true;
    };
  }, [siteKey, numericId, attempt]);

  const siteId =
    numericId ?? (resolved?.key === siteKey ? resolved.id : undefined);
  if (siteId && Number.isSafeInteger(siteId) && siteId > 0)
    return <SiteDetailPage key={siteId} siteId={siteId} onBack={onBack} />;
  if (resolved?.key === siteKey && resolved.error)
    return (
      <div className="site-route-state" role="alert">
        <h1>Detail situs tidak tersedia</h1>
        <p>{resolved.error}</p>
        <div>
          <button onClick={onBack}>Kembali ke daftar situs</button>
          <button
            onClick={() => {
              setResolved(null);
              setAttempt((value) => value + 1);
            }}
          >
            Coba lagi
          </button>
        </div>
      </div>
    );
  if (numericId !== null)
    return (
      <div role="alert">
        Alamat situs tidak valid.{" "}
        <button onClick={onBack}>Kembali ke daftar situs</button>
      </div>
    );
  return (
    <div className="site-route-state" role="status">
      Memuat detail situs...
    </div>
  );
}
