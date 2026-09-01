type Delivery = {
  endpoint: string;
  eventId: string;
  payload: unknown;
};

export async function deliverWebhook(delivery: Delivery): Promise<void> {
  const response = await fetch(delivery.endpoint, {
    body: JSON.stringify(delivery.payload),
    headers: {
      "content-type": "application/json",
      "x-event-id": delivery.eventId,
    },
    method: "POST",
  });

  if (!response.ok) {
    throw new Error(`Webhook delivery failed with ${response.status}`);
  }
}
