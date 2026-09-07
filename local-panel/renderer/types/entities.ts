export interface LocalMapping {
  id: string;
  domain: string;
  target: string;
  enabled: boolean;
  label?: string;
  workspaceId: string;
}

export interface ProxyRule {
  id: string;
  name: string;
  pattern: string;
  useRegex: boolean;
  targetType: "mapping" | "external";
  targetMappingId: string;
  targetExternal: string;
  requestScript: string;
  responseScript: string;
  enabled: boolean;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface Folder {
  id: string;
  name: string;
  parentId: string | null;
  createdAt: number;
  workspaceId: string;
}

export interface EnvVariable {
  id: string;
  key: string;
  value: string;
}

export interface Environment {
  id: string;
  name: string;
  variables: EnvVariable[];
  createdAt: number;
  workspaceId: string;
}

export interface MockRule {
  id: string;
  name: string;
  method: string;
  urlPattern: string;
  useRegex: boolean;
  enabled: boolean;
  capturedHeaders: Record<string, string>;
  capturedBody: string;       // base64
  responseStatus: number;
  responseStatusMocked?: boolean;
  responseHeaders: Record<string, string>;
  mockedResponseHeaders?: string[];
  responseBody: string;       // plain text or base64 (see responseBodyEncoding)
  responseBodyMocked?: boolean;
  responseBodyEncoding?: "utf8" | "base64";  // default "utf8"; "base64" for binary bodies
  responseDelay?: number;     // ms to wait before sending response (0 = no delay)
  responseDelayMocked?: boolean;
  streamingMode?: "none" | "sse" | "chunked";  // default "none"
  streamingChunkDelay?: number;  // ms between chunks (default 100)
  streamingChunkSeparator?: string;  // delimiter to split body into chunks
  protocol?: "rest";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export type ApiProtocol = "rest" | "graphql" | "grpc" | "soap";

export interface SavedRequest {
  id: string;
  name: string;
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;    // plain text
  protocol?: "rest";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedWsConnection {
  id: string;
  name: string;
  url: string;
  headers: Record<string, string>;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedWebhook {
  id: string;
  name: string;
  /** User-defined suffix appended after /localpanel/webhooks/ */
  urlSuffix: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedGrpcRequest {
  id: string;
  name: string;
  serverAddress: string;
  serviceName: string;
  methodName: string;
  requestBody: string;
  metadata: Record<string, string>;
  protoFileId?: string | null;
  useReflection: boolean;
  streamingType: "unary" | "server" | "client" | "bidi";
  protocol?: "grpc";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedGrpcMock {
  id: string;
  name: string;
  enabled: boolean;
  serviceName: string;
  methodName: string;
  responseBody: string;
  responseMetadata: Record<string, string>;
  responseDelay?: number;
  streamingResponses?: string[];
  errorCode?: number;
  errorMessage?: string;
  protoFileId: string;
  protocol?: "grpc";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedProtoFile {
  id: string;
  name: string;
  content: string;
  parsedServices?: { name: string; methods: { name: string; inputType: string; outputType: string; clientStreaming: boolean; serverStreaming: boolean }[] }[];
  createdAt: number;
  workspaceId: string;
}

export interface SavedSoapRequest {
  id: string;
  name: string;
  endpointUrl: string;
  soapAction: string;
  headers: Record<string, string>;
  body: string;
  wsdlId?: string | null;
  operationName?: string;
  protocol?: "soap";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedSoapMock {
  id: string;
  name: string;
  enabled: boolean;
  endpointPattern: string;
  useRegex: boolean;
  soapActionPattern: string;
  operationName?: string;
  responseStatus: number;
  responseHeaders: Record<string, string>;
  responseBody: string;
  responseDelay?: number;
  wsdlId?: string | null;
  protocol?: "soap";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedWsdl {
  id: string;
  name: string;
  content: string;
  sourceUrl?: string;
  importedAt: number;
  createdAt: number;
  workspaceId: string;
}

export interface SavedGraphQLRequest {
  id: string;
  name: string;
  endpointUrl: string;
  headers: Record<string, string>;
  query: string;
  variables: string;
  operationName: string;
  protocol?: "graphql";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  schemaId?: string | null;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedGraphQLMock {
  id: string;
  name: string;
  enabled: boolean;
  endpointPattern: string;
  useRegex: boolean;
  operationType: "query" | "mutation" | "subscription" | "any";
  operationName: string;
  responseStatus: number;
  responseHeaders: Record<string, string>;
  responseBody: string;
  responseDelay?: number;
  schemaId?: string | null;
  protocol?: "graphql";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export type UnifiedRequest =
  | (SavedRequest & { protocol?: "rest" })
  | (SavedGraphQLRequest & { protocol: "graphql" })
  | (SavedGrpcRequest & { protocol: "grpc" })
  | (SavedSoapRequest & { protocol: "soap" });

export type UnifiedMock =
  | (MockRule & { protocol?: "rest" })
  | (SavedGraphQLMock & { protocol: "graphql" })
  | (SavedGrpcMock & { protocol: "grpc" })
  | (SavedSoapMock & { protocol: "soap" });

export interface SavedGraphQLSchema {
  id: string;
  name: string;
  content: string;
  endpointUrl?: string;
  introspectedAt?: number;
  createdAt: number;
  workspaceId: string;
}
