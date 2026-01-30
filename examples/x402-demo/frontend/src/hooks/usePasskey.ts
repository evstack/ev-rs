import { useState, useCallback, useEffect } from "react";
import {
  startRegistration as webauthnRegister,
  startAuthentication as webauthnAuth,
} from "@simplewebauthn/browser";
import {
  startRegistration,
  verifyRegistration,
  startLogin,
  verifyLogin,
} from "../lib/api";

type Session = {
  token: string;
  address: string;
  username: string;
};

const SESSION_KEY = "x402_session";

function loadSession(): Session | null {
  try {
    const stored = localStorage.getItem(SESSION_KEY);
    return stored ? JSON.parse(stored) : null;
  } catch {
    return null;
  }
}

function saveSession(session: Session | null) {
  if (session) {
    localStorage.setItem(SESSION_KEY, JSON.stringify(session));
  } else {
    localStorage.removeItem(SESSION_KEY);
  }
}

export function usePasskey() {
  const [session, setSession] = useState<Session | null>(loadSession);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    saveSession(session);
  }, [session]);

  const register = useCallback(async (username: string) => {
    setLoading(true);
    setError(null);

    try {
      const { options, userId } = await startRegistration(username);
      const credential = await webauthnRegister({ optionsJSON: options });
      const result = await verifyRegistration(userId, credential);

      const newSession: Session = {
        token: result.sessionToken,
        address: result.address,
        username,
      };
      setSession(newSession);
      return newSession;
    } catch (err) {
      const message = err instanceof Error ? err.message : "Registration failed";
      setError(message);
      throw err;
    } finally {
      setLoading(false);
    }
  }, []);

  const login = useCallback(async (username?: string) => {
    setLoading(true);
    setError(null);

    try {
      const { options, authId } = await startLogin(username);
      const credential = await webauthnAuth({ optionsJSON: options });
      const result = await verifyLogin(authId, credential);

      const newSession: Session = {
        token: result.sessionToken,
        address: result.address,
        username: result.username,
      };
      setSession(newSession);
      return newSession;
    } catch (err) {
      const message = err instanceof Error ? err.message : "Login failed";
      setError(message);
      throw err;
    } finally {
      setLoading(false);
    }
  }, []);

  const logout = useCallback(() => {
    setSession(null);
    setError(null);
  }, []);

  return {
    session,
    isAuthenticated: !!session,
    loading,
    error,
    register,
    login,
    logout,
  };
}
