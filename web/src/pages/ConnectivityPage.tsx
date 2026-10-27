import { useTranslation } from 'react-i18next';
import { ConnectivitySettings } from '../components/system/ConnectivitySettings';
export function ConnectivityPage() {
  const { i18n } = useTranslation();
  return <div className="space-y-6"><h1 className="text-xl font-bold">{i18n.language.startsWith('en') ? 'Connectivity' : 'Konektivitas'}</h1><ConnectivitySettings /></div>;
}
