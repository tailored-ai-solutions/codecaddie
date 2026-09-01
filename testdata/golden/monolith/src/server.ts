import Fastify from "fastify";
import { searchDocuments } from "./search";

const server = Fastify();

server.post("/sources", async (request) => {
  return { accepted: Boolean(request.body), id: crypto.randomUUID() };
});

server.get("/search", async (request) => searchDocuments(String(request.query)));

server.listen({ port: 3000 });

