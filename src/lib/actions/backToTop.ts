//! Floating back-to-top button for long scroll containers
//!
//! Attaches to any scrollable element. The button is anchored to the
//! container's bottom-right corner (inside the container's positioned
//! parent, so it stays put while the content scrolls). It is hidden at
//! rest and only surfaces while the user is actively scrolling — once
//! scrolling pauses it fades out again, so it never sits on top of a row
//! while you're reading. Clicking it smooth-scrolls the container back to
//! the top.

import { get } from "svelte/store";
import { t } from "$lib/i18n";

const DEFAULT_THRESHOLD = 200;
/// How long after the last scroll event before the button fades out again.
const HIDE_DELAY = 1500;

export interface BackToTopOptions {
  /** Scroll distance (px) before the button is offered at all. */
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
    // Dismiss immediately; the return-to-top animation keeps it hidden.
    hide();
  });
  // Hold the button visible while the pointer is on it, so the idle fade
  // never swipes it away from under a cursor that is about to click.
  button.addEventListener("mouseenter", () => {
    if (hideTimer !== null) {
      clearTimeout(hideTimer);
      hideTimer = null;
    }
  });

  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  function hide() {
    if (hideTimer !== null) {
      clearTimeout(hideTimer);
      hideTimer = null;
    }
    button.classList.remove("visible");
  }
  const onScroll = () => {
    if (hideTimer !== null) clearTimeout(hideTimer);
    // Offer "back to top" only when meaningfully scrolled down; near the
    // top there is nothing worth jumping back to, so keep it out of the
    // way.
    if (node.scrollTop > threshold) {
      button.classList.add("visible");
      // Resurface on each scroll event, then fade out once it pauses so
      // the button never lingers over the list at rest.
      hideTimer = setTimeout(() => {
        hideTimer = null;
        button.classList.remove("visible");
      }, HIDE_DELAY);
    } else {
      hide();
    }
  };
  node.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  host.appendChild(button);

  return {
    destroy() {
      node.removeEventListener("scroll", onScroll);
      hide();
      button.remove();
      host.style.position = previousPosition;
    },
  };
}
