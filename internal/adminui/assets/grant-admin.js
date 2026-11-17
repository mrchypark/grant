document.addEventListener("change", (event) => {
  if (event.target.matches("[data-admin-scope]")) event.target.form.requestSubmit();
});

document.addEventListener("input", (event) => {
  if (!event.target.matches("[data-admin-search]")) return;
  clearTimeout(event.target.searchTimer);
  event.target.searchTimer = setTimeout(() => event.target.form.requestSubmit(), 250);
});

document.addEventListener("submit", (event) => {
  const form = event.target;
  const message = form.dataset.confirm;
  if (!message) return;
  if (form.dataset.confirmed === "true") {
    delete form.dataset.confirmed;
    return;
  }
  event.preventDefault();
  event.stopImmediatePropagation();
  if (window.confirm(message)) {
    form.dataset.confirmed = "true";
    form.requestSubmit(event.submitter);
  }
}, true);

function updateCurrentNavigation() {
  const heading = document.querySelector("#workspace h1");
  if (heading) document.title = `${heading.textContent} · Grant 관리`;
  for (const link of document.querySelectorAll(".nav-link")) {
    const active = new URL(link.href).pathname === window.location.pathname;
    link.classList.toggle("is-current", active);
    if (active) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
}

document.addEventListener("htmx:after:swap", updateCurrentNavigation);
document.addEventListener("htmx:after:history:update", updateCurrentNavigation);
document.addEventListener("htmx:after:history:restore", updateCurrentNavigation);
