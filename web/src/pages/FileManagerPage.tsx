import React from 'react';
import { FileManager } from '../components/files/FileManager';
import { TenantTarget } from '../components/tenants/TenantTarget';
import { useTranslation } from 'react-i18next';
import { FolderOpen } from 'lucide-react';
import '../styles/file-browser.css';

export interface FileManagerPageProps {
  initialPath?: string;
}

export const FileManagerPage: React.FC<FileManagerPageProps> = ({ initialPath = '' }) => {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  return (
    <div className="fb-page">
      <header className="fb-page-header"><span className="fb-page-icon"><FolderOpen size={23} aria-hidden="true" /></span><div><h1>{en ? 'File manager' : 'Pengelola Berkas'}</h1><p>{en ? 'Organize, edit and manage your project files.' : 'Susun, sunting, dan kelola berkas project Anda.'}</p></div></header>

      <TenantTarget>{tenant => <FileManager key={tenant || 'self'} tenant={tenant} initialPath={initialPath} />}</TenantTarget>
    </div>
  );
};

export default FileManagerPage;
