import React, { createContext, useContext, useEffect, useState, useCallback } from 'react';
import { User, LoginResponse } from '../types/api';
import { fetchMe, getCsrfToken, setCsrfToken, fetchCsrfToken, login as apiLogin, logout as apiLogout } from '../api/client';
import { navigate } from '../lib/router';

interface AuthContextType {
  user: User | null;
  isLoading: boolean;
  csrfToken: string | null;
  isAuthenticated: boolean;
  login: (username: string, password: string, twoFactorCode?: string) => Promise<LoginResponse>;
  logout: () => Promise<void>;
  refreshUser: () => Promise<User | null>;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [user, setUser] = useState<User | null>(null);
  const [csrfToken, setCsrfTokenState] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    const clear = () => { setUser(null); setCsrfToken(null); setCsrfTokenState(null); };
    window.addEventListener('panel:session-expired', clear);
    return () => window.removeEventListener('panel:session-expired', clear);
  }, []);

  const initAuth = useCallback(async () => {
    setIsLoading(true);
    try {
      const currentUser = await fetchMe();
      setUser(currentUser);
      setCsrfTokenState(getCsrfToken() || await fetchCsrfToken());
    } catch {
      // Not logged in or session expired
      setUser(null);
      setCsrfToken(null);
      setCsrfTokenState(null);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    initAuth();
  }, [initAuth]);

  const login = async (
    username: string,
    password: string,
    twoFactorCode?: string
  ): Promise<LoginResponse> => {
    const res = await apiLogin(username, password, twoFactorCode);
    if (res.user) {
      setUser(res.user);
    }
    if (res.csrf_token) {
      setCsrfTokenState(res.csrf_token);
    }
    return res;
  };

  const logout = async () => {
    try {
      await apiLogout();
    } finally {
      setUser(null);
      setCsrfTokenState(null);
      setCsrfToken(null);
      // An explicit sign-out starts a fresh login; expiry still retains deep links.
      navigate('/login', true);
    }
  };

  const refreshUser = async (): Promise<User | null> => {
    try {
      const u = await fetchMe();
      setUser(u);
      setCsrfTokenState(getCsrfToken() || await fetchCsrfToken());
      return u;
    } catch {
      setUser(null);
      setCsrfToken(null);
      setCsrfTokenState(null);
      return null;
    }
  };

  return (
    <AuthContext.Provider
      value={{
        user,
        isLoading,
        csrfToken,
        isAuthenticated: !!user,
        login,
        logout,
        refreshUser,
      }}
    >
      {children}
    </AuthContext.Provider>
  );
};

export const useAuth = (): AuthContextType => {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
};
