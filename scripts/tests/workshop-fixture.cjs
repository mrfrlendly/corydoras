// Install this bridge in a fresh browser context before the app loads.
function mockDesktop() {
  const info = () => ({
    library_path: '/sample/library', default_library: '/sample/library',
    config_path: '/sample/settings', mcp_command: null, recent_libraries: [],
    library_warning: null, version: 'test', inventory_enabled: sessionStorage.module === 'on',
  });
  const label = {
    name: 'Socket head screws', brand: 'Sample Hardware', part_number: 'SH-306',
    category: 'Fasteners', dimensions: 'M3 × 6 mm', quantity: null,
    pack_quantity: 25, notes: '', supplier: null,
  };
  const listing = {
    title: 'Socket head screws SH-306', url: 'https://supplier.example.com/products/sh-306',
    details: 'Stainless steel, A2 grade.',
  };
  const items = () => JSON.parse(sessionStorage.inventory || '[]');
  const store = rows => sessionStorage.setItem('inventory', JSON.stringify(rows));

  window.workshopProbe = { searches: 0, failSearch: false, commands: [] };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 1,
    invoke: async (command, args = {}) => {
      const probe = window.workshopProbe;
      probe.commands.push(command);
      switch (command) {
        case 'plugin:event|listen': return 1;
        case 'plugin:event|unlisten': return;
        case 'app_info': return info();
        case 'list_languages':
        case 'list_projects':
        case 'list_rule_sets':
        case 'db_copies': return [];
        case 'status': return {
          worker: { state: 'idle' }, current: null, queued: 0, languages: [], has_key: true,
          usage: { month: 'test', input_tokens: 0, output_tokens: 0, requests: 0 }, budget: 0,
        };
        case 'get_settings': return { debounce_secs: 5, monthly_token_budget: 0 };
        case 'get_ai': return { config: { profiles: [] }, presets: [], keys: {}, account: {} };
        case 'get_bridge': return { enabled: false, port: 47821 };
        case 'get_capture': return { language_id: null, shortcut: null };
        case 'set_inventory_enabled':
          sessionStorage.module = args.enabled ? 'on' : 'off';
          return info();
        case 'list_inventory': return items();
        case 'save_inventory': {
          const item = { ...args.draft, id: args.id ?? crypto.randomUUID() };
          store([...items().filter(row => row.id !== item.id), item]);
          return item;
        }
        case 'delete_inventory': store(items().filter(row => row.id !== args.id)); return;
        case 'read_inventory_label':
          if (new Uint8Array(args)[0] !== 137) throw new Error('Expected PNG file bytes.');
          return label;
        case 'search_inventory_listings':
          probe.searches += 1;
          if (probe.failSearch) throw new Error('Supplier connection failed.');
          if (args.partNumber !== label.part_number || args.name !== label.name || args.brand !== label.brand) {
            throw new Error('Supplier query does not match the label.');
          }
          return [listing];
        default: throw new Error('Unexpected desktop command: ' + command);
      }
    },
  };
}

module.exports = { mockDesktop };
