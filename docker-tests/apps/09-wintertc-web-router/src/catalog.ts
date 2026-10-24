import type { CatalogItem } from "./types.ts";

const CATALOG_DB: CatalogItem[] = [
  { sku: "GPU-H100-80G", name: "Hopper Tensor Core", category: "compute", priceCents: 289000, tags: ["ai", "tensor", "pcie"] },
  { sku: "GPU-B200-192G", name: "Blackwell Ultra Node", category: "compute", priceCents: 425000, tags: ["ai", "nvlink", "fp4"] },
  { sku: "NIC-CX7-400G", name: "ConnectX-7 SmartNIC", category: "network", priceCents: 18500, tags: ["rdma", "roce", "pcie"] },
  { sku: "NVME-GEN5-15T", name: "Enterprise U.3 NVMe", category: "storage", priceCents: 14200, tags: ["nvme", "pcie", "raid"] },
  { sku: "SW-IB-NDR64", name: "Quantum-2 InfiniBand", category: "network", priceCents: 96000, tags: ["rdma", "switch", "ndr"] },
  { sku: "MEM-DDR5-128G", name: "ECC Registered DIMM", category: "compute", priceCents: 6400, tags: ["ecc", "ddr5"] },
];

export class CatalogService {
  public queryCatalog(params: URLSearchParams): { items: CatalogItem[]; canonicalQuery: string } {
    const category = params.get("category") || "all";
    const minPrice = Number(params.get("minPrice") || "0");
    const requiredTags = params.getAll("tag");
    const sortOrder = params.get("sort") || "asc";

    const filtered: CatalogItem[] = [];
    for (let i = 0; i < CATALOG_DB.length; i++) {
      const item = CATALOG_DB[i];
      if (category !== "all" && item.category !== category) continue;
      if (item.priceCents < minPrice) continue;
      if (requiredTags.length > 0) {
        let allMatched = true;
        for (let t = 0; t < requiredTags.length; t++) {
          if (!item.tags.includes(requiredTags[t])) {
            allMatched = false;
            break;
          }
        }
        if (!allMatched) continue;
      }
      filtered.push(item);
    }

    filtered.sort((a, b) =>
      sortOrder === "desc" ? b.priceCents - a.priceCents : a.priceCents - b.priceCents
    );

    const normalized = new URLSearchParams();
    normalized.set("category", category);
    normalized.set("minPrice", String(minPrice));
    normalized.set("sort", sortOrder);
    for (let i = 0; i < requiredTags.length; i++) {
      normalized.append("tag", requiredTags[i]);
    }
    normalized.sort();

    return {
      items: filtered,
      canonicalQuery: normalized.toString(),
    };
  }

  public findBySku(sku: string): CatalogItem | null {
    for (let i = 0; i < CATALOG_DB.length; i++) {
      if (CATALOG_DB[i].sku === sku) return CATALOG_DB[i];
    }
    return null;
  }
}
