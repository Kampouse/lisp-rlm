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
  /** IPFS CID for P2 deployments */
  ipfsCid?: string | null;
  /** IPFS gateway URL for P2 deployments */
  ipfsUrl?: string | null;
  /** SHA256 hash of the WASM binary */
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
  }
  return connector;
}

export async function connectWallet(network: Network = 'testnet'): Promise<WalletState> {
  const conn = getConnector(network);

  // Try auto-connect first
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

  // Show wallet selector popup
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
// IPFS upload via Pinata
// ============================================
const PINATA_JWT = import.meta.env.VITE_PINATA_JWT ?? '';
const PINATA_GATEWAY = 'https://gateway.pinata.cloud/ipfs';

async function uploadToIPFS(wasmBytes: Uint8Array): Promise<{ cid: string; url: string }> {
  if (!PINATA_JWT) {
    throw new Error('IPFS upload not configured. Set VITE_PINATA_JWT in .env');
  }

  const formData = new FormData();
  formData.append('file', new Blob([wasmBytes.buffer as ArrayBuffer], { type: 'application/wasm' }), 'contract.wasm');
  formData.append(
    'pinataMetadata',
    JSON.stringify({ name: `lisp-rlm-${Date.now()}.wasm` }),
  );

  const res = await fetch('https://api.pinata.cloud/pinning/pinFileToIPFS', {
    method: 'POST',
    headers: { Authorization: `Bearer ${PINATA_JWT}` },
    body: formData,
  });

  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Pinata upload failed (${res.status}): ${text}`);
  }

  const data = await res.json();
  const cid: string = data.IpfsHash;
  return { cid, url: `${PINATA_GATEWAY}/${cid}` };
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
// P2: Deploy to OutLayer via request_execution
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

    // 2. Upload to IPFS via Pinata
    const { cid, url: ipfsUrl } = await uploadToIPFS(wasmBytes);

    // 3. Call request_execution on OutLayer with WasmUrl source
    const wallet = await conn.wallet();
    const result = await wallet.signAndSendTransaction({
      receiverId: outlayerContractId,
      actions: [
        {
          type: 'FunctionCall' as const,
          params: {
            methodName: 'request_execution',
            args: {
              source: {
                WasmUrl: {
                  url: ipfsUrl,
                  hash,
                  build_target: 'wasm32-wasip1',
                },
              },
              response_format: 'Text',
            },
            gas: '300000000000000', // 300 Tgas
            deposit: '100000000000000000000000', // 0.1 NEAR for compute
          },
        },
      ],
    });

    const txHash = result?.transaction?.hash ?? null;
    return {
      success: true,
      txHash,
      explorerUrl: toExplorerUrl(txHash, network),
      error: null,
      ipfsCid: cid,
      ipfsUrl,
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
