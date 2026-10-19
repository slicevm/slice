export type ItemCategory = "compute" | "storage" | "network" | "ai-gpu";
export type CustomerTier = "free" | "pro" | "enterprise";
export type CloudRegion = "US-EAST" | "EU-CENTRAL" | "AP-SOUTH";

export interface OrderItem {
    readonly sku: string;
    readonly title: string;
    readonly unitPriceCents: number;
    readonly quantity: number;
    readonly category: ItemCategory;
}

export interface CustomerProfile {
    readonly customerId: string;
    readonly tier: CustomerTier;
    readonly region: CloudRegion;
    readonly accountAgeDays: number;
}

export interface CheckoutRequest {
    readonly requestId: string;
    readonly customer: CustomerProfile;
    readonly items: OrderItem[];
    readonly couponCode: string | null;
}

export interface CheckoutReceipt {
    readonly requestId: string;
    readonly customerId: string;
    readonly subtotalCents: number;
    readonly discountCents: number;
    readonly taxCents: number;
    readonly finalTotalCents: number;
    readonly riskScore: number;
    readonly approved: boolean;
    readonly lineItemCount: number;
    readonly sha256Signature: string;
}

