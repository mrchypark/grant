const token =
  document.querySelector('meta[name="login-csrf-token"]')?.getAttribute('content') ?? '';
const postLoginPath =
  document.querySelector('meta[name="post-login-path"]')?.getAttribute('content') ?? '/app';
const loginForm = document.querySelector('.login-form');
const emailInput = document.getElementById('email');
const passwordInput = document.getElementById('password');
const submitButton = document.querySelector('.button--primary');
const statusNode = document.querySelector('[data-login-status]');

function setStatus(message) {
  if (statusNode) {
    statusNode.textContent = message;
  }
}

async function submitLogin() {
  if (!loginForm || !emailInput || !passwordInput || !submitButton) {
    return;
  }

  if (typeof loginForm.reportValidity === 'function' && !loginForm.reportValidity()) {
    return;
  }

  submitButton.disabled = true;
  setStatus('Signing in...');

  try {
    const response = await fetch('/api/auth/v1/login', {
      method: 'POST',
      credentials: 'same-origin',
      headers: {
        'content-type': 'application/json',
        'x-login-csrf': token,
      },
      body: JSON.stringify({
        email: emailInput.value,
        password: passwordInput.value,
      }),
    });

    if (response.ok) {
      window.location.assign(postLoginPath);
      return;
    }

    setStatus('Sign in failed.');
  } catch (_error) {
    setStatus('Network error. Try again.');
  } finally {
    submitButton.disabled = false;
  }
}

if (loginForm) {
  loginForm.addEventListener('submit', (event) => {
    event.preventDefault();
    void submitLogin();
  });
}
