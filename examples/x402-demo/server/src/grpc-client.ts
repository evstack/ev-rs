import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import { type Address, type Hash } from "viem";
import path from "path";

// Path to the proto files (relative to the evolve repo root)
const PROTO_PATH = path.resolve(
  import.meta.dir,
  "../../../../crates/rpc/grpc/proto"
);

// gRPC client types (matching the proto definitions)
interface H256 {
  data: Buffer;
}

interface AddressProto {
  data: Buffer;
}

interface U256 {
  data: Buffer;
}

interface Receipt {
  transactionHash: H256;
  transactionIndex: number;
  blockHash: H256;
  blockNumber: number;
  from: AddressProto;
  to?: AddressProto;
  cumulativeGasUsed: number;
  gasUsed: number;
  effectiveGasPrice: U256;
  contractAddress?: AddressProto;
  logs: unknown[];
  logsBloom: Buffer;
  txType: number;
  status: number; // 1 = success, 0 = failure
}

interface GetBalanceResponse {
  balance: U256;
}

interface GetTransactionReceiptResponse {
  receipt?: Receipt;
}

interface SendRawTransactionResponse {
  hash: H256;
}

interface GetTransactionCountResponse {
  count: number;
}

interface GetBlockNumberResponse {
  blockNumber: number;
}

// Service client interface
interface ExecutionServiceClient {
  getBalance(
    request: { address: AddressProto; block?: unknown },
    callback: (err: grpc.ServiceError | null, response: GetBalanceResponse) => void
  ): void;

  getTransactionReceipt(
    request: { hash: H256 },
    callback: (err: grpc.ServiceError | null, response: GetTransactionReceiptResponse) => void
  ): void;

  sendRawTransaction(
    request: { data: Buffer },
    callback: (err: grpc.ServiceError | null, response: SendRawTransactionResponse) => void
  ): void;

  getTransactionCount(
    request: { address: AddressProto; block?: unknown },
    callback: (err: grpc.ServiceError | null, response: GetTransactionCountResponse) => void
  ): void;

  getBlockNumber(
    request: Record<string, never>,
    callback: (err: grpc.ServiceError | null, response: GetBlockNumberResponse) => void
  ): void;
}

// Helper functions for type conversion
function addressToProto(address: Address): AddressProto {
  return { data: Buffer.from(address.slice(2), "hex") };
}

function hashToProto(hash: Hash): H256 {
  return { data: Buffer.from(hash.slice(2), "hex") };
}

function protoToHash(h256: H256): Hash {
  return `0x${Buffer.from(h256.data).toString("hex")}` as Hash;
}

function protoToU256(u256: U256): bigint {
  if (!u256.data || u256.data.length === 0) return 0n;
  return BigInt(`0x${Buffer.from(u256.data).toString("hex")}`);
}

// Typed client wrapper
export interface EvolveGrpcClient {
  getBalance(address: Address): Promise<bigint>;
  getTransactionReceipt(hash: Hash): Promise<{
    success: boolean;
    blockNumber: number;
    gasUsed: number;
    from: Address;
    to?: Address;
  } | null>;
  sendRawTransaction(signedTx: `0x${string}`): Promise<Hash>;
  getTransactionCount(address: Address): Promise<number>;
  getBlockNumber(): Promise<number>;
  close(): void;
}

export async function createEvolveGrpcClient(
  endpoint: string = "localhost:9545"
): Promise<EvolveGrpcClient> {
  // Load proto files
  const packageDefinition = await protoLoader.load(
    [
      path.join(PROTO_PATH, "evolve/v1/execution.proto"),
      path.join(PROTO_PATH, "evolve/v1/types.proto"),
    ],
    {
      keepCase: false, // Convert to camelCase
      longs: Number,
      enums: String,
      defaults: true,
      oneofs: true,
      includeDirs: [PROTO_PATH],
    }
  );

  const protoDescriptor = grpc.loadPackageDefinition(packageDefinition);
  const evolve = protoDescriptor.evolve as { v1: { ExecutionService: grpc.ServiceClientConstructor } };

  // Create client
  const client = new evolve.v1.ExecutionService(
    endpoint,
    grpc.credentials.createInsecure()
  ) as unknown as ExecutionServiceClient;

  return {
    async getBalance(address: Address): Promise<bigint> {
      return new Promise((resolve, reject) => {
        client.getBalance(
          { address: addressToProto(address) },
          (err, response) => {
            if (err) reject(err);
            else resolve(protoToU256(response.balance));
          }
        );
      });
    },

    async getTransactionReceipt(hash: Hash) {
      return new Promise((resolve, reject) => {
        client.getTransactionReceipt({ hash: hashToProto(hash) }, (err, response) => {
          if (err) reject(err);
          else if (!response.receipt) resolve(null);
          else {
            const r = response.receipt;
            resolve({
              success: r.status === 1,
              blockNumber: r.blockNumber,
              gasUsed: r.gasUsed,
              from: `0x${Buffer.from(r.from.data).toString("hex")}` as Address,
              to: r.to ? (`0x${Buffer.from(r.to.data).toString("hex")}` as Address) : undefined,
            });
          }
        });
      });
    },

    async sendRawTransaction(signedTx: `0x${string}`): Promise<Hash> {
      return new Promise((resolve, reject) => {
        client.sendRawTransaction(
          { data: Buffer.from(signedTx.slice(2), "hex") },
          (err, response) => {
            if (err) reject(err);
            else resolve(protoToHash(response.hash));
          }
        );
      });
    },

    async getTransactionCount(address: Address): Promise<number> {
      return new Promise((resolve, reject) => {
        client.getTransactionCount(
          { address: addressToProto(address) },
          (err, response) => {
            if (err) reject(err);
            else resolve(response.count);
          }
        );
      });
    },

    async getBlockNumber(): Promise<number> {
      return new Promise((resolve, reject) => {
        client.getBlockNumber({}, (err, response) => {
          if (err) reject(err);
          else resolve(response.blockNumber);
        });
      });
    },

    close() {
      grpc.closeClient(client as unknown as grpc.Client);
    },
  };
}

// Singleton instance for the server
let grpcClientInstance: EvolveGrpcClient | null = null;

export async function getGrpcClient(): Promise<EvolveGrpcClient> {
  if (!grpcClientInstance) {
    const endpoint = process.env.EVOLVE_GRPC_URL ?? "localhost:9545";
    grpcClientInstance = await createEvolveGrpcClient(endpoint);
  }
  return grpcClientInstance;
}

export function closeGrpcClient(): void {
  if (grpcClientInstance) {
    grpcClientInstance.close();
    grpcClientInstance = null;
  }
}
