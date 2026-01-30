import {
  generateRegistrationOptions,
  verifyRegistrationResponse,
  generateAuthenticationOptions,
  verifyAuthenticationResponse,
  type VerifiedRegistrationResponse,
  type VerifiedAuthenticationResponse,
} from "@simplewebauthn/server";
import type {
  RegistrationResponseJSON,
  AuthenticationResponseJSON,
  PublicKeyCredentialCreationOptionsJSON,
  PublicKeyCredentialRequestOptionsJSON,
  AuthenticatorTransportFuture,
} from "@simplewebauthn/types";
import { Hono, type MiddlewareHandler } from "hono";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { type Address, type Hex } from "viem";

// Relying Party configuration
const RP_NAME = "X402 Demo";
const RP_ID = process.env.RP_ID ?? "localhost";
const RP_ORIGIN = process.env.RP_ORIGIN ?? "http://localhost:5173";

// In-memory stores (use Redis/SQLite in production)
type StoredCredential = {
  credentialId: string; // base64url encoded
  credentialPublicKey: Uint8Array;
  counter: number;
  transports?: AuthenticatorTransportFuture[];
};

export type UserRecord = {
  id: string;
  username: string;
  credentials: StoredCredential[];
  // Server-managed signing key (secp256k1)
  privateKey: Hex;
  address: Address;
};

// Stores
const users = new Map<string, UserRecord>();
const sessions = new Map<string, { userId: string; expiresAt: number }>();
const pendingRegistrations = new Map<string, { challenge: string; userId: string; username: string }>();
const pendingAuthentications = new Map<string, { challenge: string; userId: string }>();

/**
 * Generate a random user ID
 */
function generateUserId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Generate a session token
 */
function generateSessionToken(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Create a new server-managed wallet for the user
 */
function createServerWallet(): { privateKey: Hex; address: Address } {
  const privateKey = generatePrivateKey();
  const account = privateKeyToAccount(privateKey);
  return { privateKey, address: account.address };
}

/**
 * Get user by credential ID (base64url encoded)
 */
function getUserByCredentialId(credentialId: string): UserRecord | undefined {
  for (const user of users.values()) {
    const found = user.credentials.find((c) => c.credentialId === credentialId);
    if (found) return user;
  }
  return undefined;
}

/**
 * Get user by username
 */
function getUserByUsername(username: string): UserRecord | undefined {
  for (const user of users.values()) {
    if (user.username === username) return user;
  }
  return undefined;
}

/**
 * Validate session token and return user
 */
export function validateSession(token: string): UserRecord | null {
  const session = sessions.get(token);
  if (!session) return null;
  if (Date.now() > session.expiresAt) {
    sessions.delete(token);
    return null;
  }
  return users.get(session.userId) ?? null;
}

/**
 * Auth middleware - validates Bearer token
 */
export function authMiddleware(): MiddlewareHandler {
  return async (c, next) => {
    const authHeader = c.req.header("Authorization");
    if (!authHeader?.startsWith("Bearer ")) {
      return c.json({ error: "Authorization required" }, 401);
    }

    const token = authHeader.slice(7);
    const user = validateSession(token);
    if (!user) {
      return c.json({ error: "Invalid or expired session" }, 401);
    }

    c.set("user", user);
    return next();
  };
}

export function createPasskeyRoutes() {
  const app = new Hono();

  /**
   * POST /auth/register
   * Start registration - generate WebAuthn options
   */
  app.post("/register", async (c) => {
    const body = await c.req.json<{ username: string }>();

    if (!body.username || body.username.length < 3) {
      return c.json({ error: "Username must be at least 3 characters" }, 400);
    }

    // Check if username already exists
    if (getUserByUsername(body.username)) {
      return c.json({ error: "Username already taken" }, 400);
    }

    const userId = generateUserId();
    const userIdBytes = new TextEncoder().encode(userId);

    const options: PublicKeyCredentialCreationOptionsJSON = await generateRegistrationOptions({
      rpName: RP_NAME,
      rpID: RP_ID,
      userID: userIdBytes,
      userName: body.username,
      userDisplayName: body.username,
      attestationType: "none",
      authenticatorSelection: {
        residentKey: "preferred",
        userVerification: "preferred",
        authenticatorAttachment: "platform",
      },
      supportedAlgorithmIDs: [-7, -257], // ES256, RS256
    });

    // Store pending registration
    pendingRegistrations.set(userId, {
      challenge: options.challenge,
      userId,
      username: body.username,
    });

    return c.json({
      options,
      userId,
    });
  });

  /**
   * POST /auth/register/verify
   * Complete registration - verify attestation and create account
   */
  app.post("/register/verify", async (c) => {
    const body = await c.req.json<{
      userId: string;
      credential: RegistrationResponseJSON;
    }>();

    const pending = pendingRegistrations.get(body.userId);
    if (!pending) {
      return c.json({ error: "No pending registration" }, 400);
    }

    let verification: VerifiedRegistrationResponse;
    try {
      verification = await verifyRegistrationResponse({
        response: body.credential,
        expectedChallenge: pending.challenge,
        expectedOrigin: RP_ORIGIN,
        expectedRPID: RP_ID,
      });
    } catch (err) {
      console.error("Registration verification failed:", err);
      return c.json({ error: "Verification failed" }, 400);
    }

    if (!verification.verified || !verification.registrationInfo) {
      return c.json({ error: "Verification failed" }, 400);
    }

    const { credential } = verification.registrationInfo;

    // Create server-managed wallet
    const wallet = createServerWallet();

    // Create user record
    const user: UserRecord = {
      id: body.userId,
      username: pending.username,
      credentials: [
        {
          credentialId: body.credential.id, // Use the base64url string from client
          credentialPublicKey: credential.publicKey,
          counter: credential.counter,
          transports: body.credential.response.transports,
        },
      ],
      privateKey: wallet.privateKey,
      address: wallet.address,
    };

    users.set(body.userId, user);
    pendingRegistrations.delete(body.userId);

    // Create session
    const sessionToken = generateSessionToken();
    sessions.set(sessionToken, {
      userId: body.userId,
      expiresAt: Date.now() + 24 * 60 * 60 * 1000, // 24 hours
    });

    return c.json({
      success: true,
      address: user.address,
      sessionToken,
    });
  });

  /**
   * POST /auth/login
   * Start authentication - generate WebAuthn options
   */
  app.post("/login", async (c) => {
    const body = await c.req.json<{ username?: string }>();

    let allowCredentials: { id: string; transports?: AuthenticatorTransportFuture[] }[] | undefined;
    let userId: string | undefined;

    if (body.username) {
      const user = getUserByUsername(body.username);
      if (!user) {
        return c.json({ error: "User not found" }, 404);
      }
      userId = user.id;
      allowCredentials = user.credentials.map((cred) => ({
        id: cred.credentialId,
        transports: cred.transports,
      }));
    }

    const options: PublicKeyCredentialRequestOptionsJSON = await generateAuthenticationOptions({
      rpID: RP_ID,
      userVerification: "preferred",
      allowCredentials,
    });

    // Store pending authentication
    const authId = generateUserId();
    pendingAuthentications.set(authId, {
      challenge: options.challenge,
      userId: userId ?? "",
    });

    return c.json({
      options,
      authId,
    });
  });

  /**
   * POST /auth/login/verify
   * Complete authentication - verify assertion
   */
  app.post("/login/verify", async (c) => {
    const body = await c.req.json<{
      authId: string;
      credential: AuthenticationResponseJSON;
    }>();

    const pending = pendingAuthentications.get(body.authId);
    if (!pending) {
      return c.json({ error: "No pending authentication" }, 400);
    }

    // Find user by credential ID (base64url string)
    const user = getUserByCredentialId(body.credential.id);

    if (!user) {
      return c.json({ error: "Credential not found" }, 404);
    }

    const storedCredential = user.credentials.find(
      (cred) => cred.credentialId === body.credential.id
    );

    if (!storedCredential) {
      return c.json({ error: "Credential not found" }, 404);
    }

    let verification: VerifiedAuthenticationResponse;
    try {
      verification = await verifyAuthenticationResponse({
        response: body.credential,
        expectedChallenge: pending.challenge,
        expectedOrigin: RP_ORIGIN,
        expectedRPID: RP_ID,
        credential: {
          id: storedCredential.credentialId,
          publicKey: storedCredential.credentialPublicKey,
          counter: storedCredential.counter,
          transports: storedCredential.transports,
        },
      });
    } catch (err) {
      console.error("Authentication verification failed:", err);
      return c.json({ error: "Verification failed" }, 400);
    }

    if (!verification.verified) {
      return c.json({ error: "Verification failed" }, 400);
    }

    // Update counter
    storedCredential.counter = verification.authenticationInfo.newCounter;

    pendingAuthentications.delete(body.authId);

    // Create session
    const sessionToken = generateSessionToken();
    sessions.set(sessionToken, {
      userId: user.id,
      expiresAt: Date.now() + 24 * 60 * 60 * 1000, // 24 hours
    });

    return c.json({
      success: true,
      address: user.address,
      username: user.username,
      sessionToken,
    });
  });

  /**
   * GET /auth/me
   * Get current user info (requires auth)
   */
  app.get("/me", authMiddleware(), (c) => {
    const user = (c as any).get("user") as UserRecord;
    return c.json({
      username: user.username,
      address: user.address,
      credentialCount: user.credentials.length,
    });
  });

  /**
   * POST /auth/logout
   * Invalidate session
   */
  app.post("/logout", (c) => {
    const authHeader = c.req.header("Authorization");
    if (authHeader?.startsWith("Bearer ")) {
      const token = authHeader.slice(7);
      sessions.delete(token);
    }
    return c.json({ success: true });
  });

  return app;
}

/**
 * Get user's private key for signing (internal use only)
 */
export function getUserPrivateKey(user: UserRecord): Hex {
  return user.privateKey;
}
