export async function searchDocuments(query: string) {
  return [{ id: "document-1", query, score: 0.92 }];
}

