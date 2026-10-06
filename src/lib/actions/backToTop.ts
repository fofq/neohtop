//! Floating back-to-top button for long scroll containers
//!
//! Attaches to any scrollable element. The button is anchored to the
//! container's bottom-right corner (inside the container's positioned
//! parent, so it stays put while the content scrolls), only appears once
//! the container has scrolled past a threshold, sits at low opacity while
//! the mouse is elsewhere and lights up on hover, and smooth-scrolls the
//! container back to the top on click.

import { get } from "svelte/store";
import { t } from "$lib/i18n";

const DEFAULT_THRESHOLD = 200;

export interface BackToTopOptions {
  /** Scroll distance (px) before the button shows up. */
  threshold?: number;
}

export function backToTop(
  node: HTMLElement,
  options: BackToTopOptions = {},
): { destroy: () => void } {
  const threshold = options.threshold ?? DEFAULT_THRESHOLD;

  const host = node.parentElement;
  if (!host) {
    return { destroy: () => {} };
  }
  const previousPosition = host.style.position;
  if (getComputedStyle(host).position === "static") {
    // The absolutely-positioned button needs this ancestor as its
    // containing block; the host wraps exactly the visible area, so the
    // button stays fixed to the viewport portion while content scrolls.
    host.style.position = "relative";
  }

  const button = document.createElement("button");
  button.type = "button";
  button.className = "back-to-top";
  const label = get(t)("common.backToTop");
  button.title = label;
  button.setAttribute("aria-label", label);
  button.innerHTML =
    '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5"/><path d="M5 12l7-7 7 7"/></svg>';
  button.addEventListener("click", () => {
    node.scrollTo({ top: 0, behavior: "smooth" });
  });

  let shown = false;
  const sync = () => {
    const shouldShow = node.scrollTop > threshold;
    if (shouldShow !== shown) {
      shown = shouldShow;
      button.classList.toggle("visible", shown);
    }
  };
  node.addEventListener("scroll", sync, { passive: true });
  sync();

  host.appendChild(button);

  return {
    destroy() {
      node.removeEventListener("scroll", sync);
      button.remove();
      host.style.position = previousPosition;
    },
  };
}
