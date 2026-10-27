import React from 'react';
import { FileManager } from '../components/files/FileManager';
import { TenantTarget } from '../components/tenants/TenantTarget';

export interface FileManagerPageProps {
  initialPath?: string;
}

export const FileManagerPage: React.FC<FileManagerPageProps> = ({ initialPath = '' }) => {
  return (
    <div className="p-4 sm:p-6 space-y-4 max-w-[1400px]">
      <div>
        <h1 className="text-xl font-bold tracking-tight text-[var(--foreground)]">
          Pengelola Berkas
        </h1>
        <p className="text-xs text-[var(--muted-foreground)]">
          Jelajahi, sunting, unggah, dan kelola seluruh berkas hosting dalam ruang terisolasi tenant
        </p>
      </div>

      <TenantTarget>{tenant => <FileManager key={tenant || 'self'} tenant={tenant} initialPath={initialPath} />}</TenantTarget>
    </div>
  );
};

export default FileManagerPage;
