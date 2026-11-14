import { useState } from 'react';
import { ExternalLink, Search } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { errText } from '../api';
import type { SupplierListing } from '../inventoryApi';
import { useToast } from './Toast';
import { publicListingUrl } from './workshopForm';

export type SearchState =
  | { status: 'idle' | 'pending' }
  | { status: 'done'; results: SupplierListing[] }
  | { status: 'error'; error: string };

interface Props {
  selected: SupplierListing | null;
  state: SearchState;
  query: string;
  canSearch: boolean;
  manual: boolean;
  setManual: (show: boolean) => void;
  onSearch: () => void;
  onSelect: (listing: SupplierListing | null) => void;
}

function ListingCard({ listing, action, onClick }: { listing: SupplierListing; action: string; onClick: () => void }) {
  const toast = useToast();
  const url = publicListingUrl(listing.url);
  return <div className="workshop-listing">
    <strong>{listing.title}</strong>
    {url && <button type="button" className="workshop-source link-btn" title={url.href} aria-label={`Open ${listing.title} on ${url.hostname}`}
      onClick={() => openUrl(url.href).catch(e => toast.error(errText(e)))}>
      {url.hostname} <ExternalLink size={12} aria-hidden="true" />
    </button>}
    {listing.details && <p className="workshop-details selectable">{listing.details}</p>}
    <button type="button" className="btn small" onClick={onClick}>{action}</button>
  </div>;
}

export default function WorkshopSupplier({ selected, query, canSearch, state, manual, setManual, onSearch, onSelect }: Props) {
  const toast = useToast();
  const [link, setLink] = useState<SupplierListing>({ url: '', title: '', details: '' });
  const [error, setError] = useState('');
  const ready = query.trim().length > 0;
  function attach() {
    const url = publicListingUrl(link.url.trim());
    if (!url || !link.title.trim()) {
      setError('Enter a title and a public HTTPS address for the product.');
      return;
    }
    onSelect({ title: link.title.trim(), url: url.href, details: link.details.trim() });
    setManual(false);
    setLink({ title: '', url: '', details: '' });
    setError('');
  }

  return <section className="panel workshop-supplier" aria-labelledby="supplier-heading">
    <h3 id="supplier-heading">Supplier listing</h3>
    {selected && <ListingCard listing={selected} action="Remove listing" onClick={() => onSelect(null)} />}
    <p className="small muted">Search Canada Bolts, 3D Printing Canada, and The Pi Hut. Results can omit unavailable suppliers. Check the product and variant before you attach a listing.</p>
    <div className="row">
      <button type="button" className="btn small" disabled={!canSearch || state.status === 'pending'} onClick={onSearch}>
        <Search size={13} aria-hidden="true" /> Search suppliers
      </button>
      {ready && <button type="button" className="link-btn small" onClick={() =>
        openUrl(`https://www.amazon.com/s?k=${encodeURIComponent(query)}`).catch(e => toast.error(errText(e)))}>
        Search Amazon <ExternalLink size={11} aria-hidden="true" />
      </button>}
      <button type="button" className="link-btn small" aria-expanded={manual} onClick={() => { setManual(!manual); setError(''); }}>
        {manual ? 'Cancel link entry' : 'Enter a listing link'}
      </button>
    </div>
    {manual && <div className="workshop-manual-link">
      <label className="field"><span>Listing title</span><input value={link.title} maxLength={512}
        onChange={e => setLink({ ...link, title: e.target.value })} /></label>
      <label className="field"><span>Listing URL</span><input type="text" inputMode="url" placeholder="https://" value={link.url} maxLength={4096}
        onChange={e => setLink({ ...link, url: e.target.value })} /></label>
      <label className="field"><span>Details from the listing</span><textarea value={link.details} maxLength={8192} rows={2}
        onChange={e => setLink({ ...link, details: e.target.value })} /></label>
      {error && <p className="error-text" role="alert">{error}</p>}
      <button type="button" className="btn small" onClick={attach}>Attach listing</button>
    </div>}
    {state.status === 'pending' && <p role="status" className="small">Supplier search in progress…</p>}
    {state.status === 'error' && <p role="alert" className="error-text small">Supplier search failed. {state.error} You can still save this item.</p>}
    {state.status === 'done' && <>
      <p role="status" className="small">{state.results.length === 0 ? 'No supplier listings found.' :
        `${state.results.length} possible ${state.results.length === 1 ? 'match' : 'matches'}.`}</p>
      {state.results.filter(listing => listing.url !== selected?.url).map(listing =>
        <ListingCard key={listing.url} listing={listing} action="Use listing" onClick={() => onSelect(listing)} />)}
    </>}
  </section>;
}
