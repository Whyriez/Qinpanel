import { Toaster as SonnerToaster } from 'sonner';

export const ToastProvider: React.FC = () => {
  return (
    <SonnerToaster
      position="bottom-right"
      toastOptions={{
        className: 'border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] shadow-[var(--shadow-floating)] rounded-[var(--radius-md)] text-xs',
        style: {
          background: 'var(--surface)',
          color: 'var(--ink)',
          borderColor: 'var(--line)',
        },
      }}
    />
  );
};
