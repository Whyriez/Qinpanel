import { useTranslation } from 'react-i18next';
import { RouteLink } from '../lib/router';

export function NotFoundPage() {
  const { t } = useTranslation();
  return (
    <main className="mx-auto flex min-h-[55vh] w-full max-w-3xl flex-col justify-center px-4 py-12 sm:px-8" aria-labelledby="not-found-title">
      <p className="mb-6 text-7xl font-bold tracking-tight text-[var(--primary)]" aria-label="Error 404">404</p>
      <h1 id="not-found-title" className="mb-4 text-3xl font-semibold tracking-tight text-[var(--ink)]">{t('notFound.title')}</h1>
      <p className="max-w-lg leading-relaxed text-[var(--ink-muted)]">{t('notFound.description')}</p>
      <div className="mt-8 flex flex-wrap gap-3">
        <RouteLink href="/dashboard" className="rounded-lg bg-[var(--primary)] px-5 py-3 font-semibold text-[var(--primary-foreground)] focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[var(--primary)]">{t('notFound.dashboard')}</RouteLink>
        <RouteLink href="/sites" className="rounded-lg border border-[var(--line)] px-5 py-3 font-medium text-[var(--ink)] focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[var(--primary)]">{t('notFound.sites')}</RouteLink>
      </div>
    </main>
  );
}
