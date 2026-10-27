import { useTranslation } from "react-i18next";

export function Brand() {
  const { t } = useTranslation();
  return (
    <div className="panel-brand" aria-label="Qinpanel">
      <img className="brand-mark" src="/brand/qinpanel-mark.svg?v=4" width="40" height="40" alt="" aria-hidden="true" />
      <span className="brand-name">
        Qinpanel
        <span className="brand-description">{t("design.hostingPanel")}</span>
      </span>
    </div>
  );
}

export function BrandLoading({ message, fullScreen = false }: { message: string; fullScreen?: boolean }) {
  return <div className={`panel-loading brand-loading ${fullScreen ? 'panel-loading--screen brand-loading--full' : 'panel-loading--page'}`} role="status" aria-live="polite" aria-atomic="true">
    <div className="panel-loading__content">
      {fullScreen && <Brand />}
      <div className="panel-loading__status">
        <span className="panel-loading__spinner" aria-hidden="true" />
        <p>{message}</p>
      </div>
      {!fullScreen && <div className="panel-loading__placeholder" aria-hidden="true"><span /><span /><span /></div>}
    </div>
  </div>;
}
