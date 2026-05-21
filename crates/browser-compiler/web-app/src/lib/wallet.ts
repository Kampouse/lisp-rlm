import { NearConnector } from '@hot-labs/near-connect';

export type Network = 'testnet' | 'mainnet';

export interface WalletState {
  connected: boolean;
  accountId: string | null;
  network: Network;
}

export interface DeployResult {
  success: boolean;
  txHash: string | null;
  explorerUrl: string | null;
  error: string | null;
  /** FastFS URL for P2 deployments */
  fastfsUrl?: string | null;
  /** SHA-256 hash of the WASM binary */
  wasmHash?: string | null;
}

let connector: NearConnector | null = null;
let connectorNetwork: Network | null = null;
let currentAccountId: string | null = null;

export function getConnector(network: Network = 'testnet'): NearConnector {
  if (!connector || connectorNetwork !== network) {
    connector = new NearConnector({ network });
    connectorNetwork = network;
    connector.on('wallet:signIn', (t: any) => {
      currentAccountId = t.accounts?.[0]?.accountId ?? null;
    });
    connector.on('wallet:signOut', () => {
      currentAccountId = null;
    });
    // Register passkey wallet as a custom sandbox wallet
    registerPasskeyWallet(connector);
  }
  return connector;
}

async function registerPasskeyWallet(conn: NearConnector) {
  try {
    await conn.registerWallet({
      id: 'passkey-wallet',
      platform: ['web'],
      name: 'Passkey Wallet',
      icon: 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" rx="20" fill="url(#g)"/><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" stop-color="#00c08b"/><stop offset="100%" stop-color="#00a878"/></linearGradient></defs><text x="50" y="62" font-size="40" text-anchor="middle" fill="white">🔑</text></svg>'),
      description: 'NEAR passkey wallet — FaceID / fingerprint',
      website: 'https://near-passkey-wallet.pages.dev',
      version: '1.0.0',
      executor: 'https://de533f90.near-passkey-wallet.pages.dev/executor.js',
      type: 'sandbox',
      permissions: {
        storage: true,
        external: [],
        allowsOpen: ['https://de533f90.near-passkey-wallet.pages.dev', 'https://near-passkey-wallet.pages.dev'],
      },
      features: {
        signMessage: false,
        signTransaction: false,
        signAndSendTransaction: true,
        signAndSendTransactions: true,
        signInWithoutAddKey: true,
        signInAndSignMessage: false,
        signInWithFunctionCallKey: false,
        signDelegateActions: false,
        mainnet: true,
        testnet: true,
      },
    });
  } catch (e) {
    console.warn('Failed to register passkey wallet:', e);
  }
}

export async function connectWallet(network: Network = 'testnet'): Promise<WalletState> {
  const conn = getConnector(network);

  try {
    const wallet = await conn.wallet();
    const accounts = await wallet.getAccounts();
    if (accounts.length > 0) {
      currentAccountId = accounts[0].accountId;
      return { connected: true, accountId: currentAccountId, network };
    }
  } catch {
    // Not signed in yet
  }

  try {
    const wallet = await conn.connect();
    const accounts = await wallet.getAccounts();
    if (accounts.length > 0) {
      currentAccountId = accounts[0].accountId;
      return { connected: true, accountId: currentAccountId, network };
    }
  } catch (err: unknown) {
    console.error('Wallet connection failed:', err);
  }

  return { connected: false, accountId: null, network };
}

export async function disconnectWallet(): Promise<void> {
  const conn = getConnector();
  try {
    await conn.disconnect();
  } catch {
    // ignore
  }
  currentAccountId = null;
}

export function getWalletState(): WalletState {
  return {
    connected: currentAccountId !== null,
    accountId: currentAccountId,
    network: 'mainnet',
  };
}

// ============================================
// SHA-256 hash (Web Crypto API)
// ============================================
async function sha256(bytes: Uint8Array): Promise<string> {
  const hashBuffer = await crypto.subtle.digest('SHA-256', bytes.buffer as ArrayBuffer);
  return Array.from(new Uint8Array(hashBuffer))
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

// ============================================
// P1: Deploy as NEAR smart contract
// ============================================
export async function deployP1(
  wasmBytes: Uint8Array,
  contractName: string,
  network: Network = 'testnet',
): Promise<DeployResult> {
  if (!currentAccountId) {
    return { success: false, txHash: null, explorerUrl: null, error: 'Wallet not connected' };
  }

  const conn = getConnector(network);
  const subAccountId = `${contractName}.${currentAccountId}`;

  try {
    const wallet = await conn.wallet();
    const result = await wallet.signAndSendTransactions({
      transactions: [
        {
          receiverId: subAccountId,
          actions: [
            { type: 'CreateAccount' },
            { type: 'DeployContract' as const, params: { code: wasmBytes } },
          ],
        },
      ],
    });

    const txHash = result?.[0]?.transaction?.hash ?? null;
    return {
      success: true,
      txHash,
      explorerUrl: toExplorerUrl(txHash, network),
      error: null,
    };
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    return { success: false, txHash: null, explorerUrl: null, error: message };
  }
}

// ============================================
// P2: Deploy to OutLayer via FastFS + request_execution
// ============================================
export async function deployP2(
  wasmBytes: Uint8Array,
  outlayerContractId: string,
  network: Network = 'testnet',
): Promise<DeployResult> {
  if (!currentAccountId) {
    return { success: false, txHash: null, explorerUrl: null, error: 'Wallet not connected' };
  }

  const conn = getConnector(network);

  try {
    // 1. Compute SHA-256 hash of the WASM binary
    const hash = await sha256(wasmBytes);

    // 2. Upload to FastFS via OutLayer contract (__fastdata_fastfs)
    const wallet = await conn.wallet();

    // Encode args as JSON { data: base64, mime_type, filename }
    const base64Data = btoa(
      Array.from(new Uint8Array(wasmBytes.buffer as ArrayBuffer))
        .map((b) => String.fromCharCode(b))
        .join(''),
    );
    const uploadArgs = JSON.stringify({
      data: base64Data,
      mime_type: 'application/wasm',
      filename: `${hash}.wasm`,
    });

    const uploadResult = await wallet.signAndSendTransaction({
      receiverId: outlayerContractId,
      actions: [
        {
          type: 'FunctionCall' as const,
          params: {
            methodName: '__fastdata_fastfs',
            args: uploadArgs,
            gas: '300000000000000', // 300 Tgas
            deposit: '0',
          },
        },
      ],
    });

    const uploadTxHash = uploadResult?.transaction?.hash ?? null;

    // 3. Build FastFS URL
    const fastfsUrl = `https://main.fastfs.io/${currentAccountId}/${outlayerContractId}/${hash}.wasm`;

    // 4. Call request_execution with WasmUrl source
    const execArgs = JSON.stringify({
      source: {
        WasmUrl: {
          url: fastfsUrl,
          hash,
          build_target: 'wasm32-wasip2',
        },
      },
      response_format: 'Text',
    });

    const execResult = await wallet.signAndSendTransaction({
      receiverId: outlayerContractId,
      actions: [
        {
          type: 'FunctionCall' as const,
          params: {
            methodName: 'request_execution',
            args: execArgs,
            gas: '300000000000000',
            deposit: '100000000000000000000000', // 0.1 NEAR
          },
        },
      ],
    });

    const txHash = execResult?.transaction?.hash ?? uploadTxHash;
    return {
      success: true,
      txHash,
      explorerUrl: toExplorerUrl(txHash, network),
      error: null,
      fastfsUrl,
      wasmHash: hash,
    };
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    return { success: false, txHash: null, explorerUrl: null, error: message };
  }
}

export function toExplorerUrl(txHash: string | null, network: Network = 'testnet'): string | null {
  if (!txHash) return null;
  const prefix = network === 'mainnet' ? '' : `${network}.`;
  return `https://explorer.${prefix}near.org/transactions/${txHash}`;
}
