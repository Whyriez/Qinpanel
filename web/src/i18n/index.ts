import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import id from './locales/id.json';
import en from './locales/en.json';
import { designId, designEn } from './design';

i18n.on('languageChanged', (language: string) => {
  if (typeof document !== 'undefined') document.documentElement.lang = language;
});

i18n
  .use(initReactI18next)
  .init({
    resources: {
      id: { translation: { ...id, design: designId } },
      en: { translation: { ...en, design: designEn } },
    },
    lng: 'id', // Default language is Indonesian as specified in PRD
    fallbackLng: 'id',
    interpolation: {
      escapeValue: false,
    },
  });

export default i18n;
