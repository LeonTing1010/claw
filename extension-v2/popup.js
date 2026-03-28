const statusEl = document.getElementById('status')
const clawsEl = document.getElementById('claws')

chrome.runtime.sendMessage({ action: 'list' }, (response) => {
  if (!response || response.error) {
    statusEl.textContent = 'error: ' + (response?.error || 'no response')
    return
  }

  const claws = response.claws || []
  statusEl.textContent = `${claws.length} claws loaded`
  statusEl.classList.add('connected')

  if (claws.length === 0) {
    clawsEl.innerHTML = '<li class="empty">no claws registered</li>'
    return
  }

  for (const claw of claws) {
    const li = document.createElement('li')
    li.innerHTML = `<span class="site">${claw.site}</span>/<span class="name">${claw.name}</span><span class="desc">${claw.description}</span>`
    li.addEventListener('click', () => {
      chrome.runtime.sendMessage({ action: 'run', site: claw.site, name: claw.name }, (result) => {
        console.log(`claw://${claw.site}/${claw.name}`, result)
      })
    })
    clawsEl.appendChild(li)
  }
})
