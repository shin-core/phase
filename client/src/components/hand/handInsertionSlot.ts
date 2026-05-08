interface HandSlotRect {
  objectId: number;
  left: number;
  width: number;
}

export function computeHandInsertionSlot(
  cards: HandSlotRect[],
  clientX: number,
  draggingId: number,
): number | null {
  if (cards.length === 0) return null;

  const remaining = cards.filter((card) => card.objectId !== draggingId);
  for (let slot = 0; slot < remaining.length; slot++) {
    const card = remaining[slot];
    const center = card.left + card.width / 2;
    if (clientX < center) return slot;
  }

  return remaining.length;
}
