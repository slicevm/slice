export interface CatalogItem {
  sku: string;
  name: string;
  category: string;
  priceCents: number;
  tags: string[];
}

export interface OrderRequestPayload {
  sku: string;
  quantity: number;
  customerTier: string;
}

export interface RouterBenchmarkSummary {
  app: string;
  totalRequests: number;
  ok200Count: number;
  created201Count: number;
  redirect308Count: number;
  unauthorized401Count: number;
  notFound404Count: number;
  totalRevenueCents: number;
  matchedCatalogItems: number;
  canonicalQuerySample: string;
  finalStateHash: string;
}

