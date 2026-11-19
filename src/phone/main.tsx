import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import PhoneApp from './PhoneApp';
import './phone.css';

createRoot(document.getElementById('phone-root')!).render(<StrictMode><PhoneApp /></StrictMode>);
