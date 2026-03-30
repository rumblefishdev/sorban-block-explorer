// --- Pagination types ---

export interface PaginationRequest {
  cursor: string | null;
  limit: number;
}

export interface PaginatedResponse<T> {
  data: readonly T[];
  nextCursor: string | null;
  hasMore: boolean;
}
