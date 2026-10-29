import '../styles/tokens.css';
import '../styles/base.css';
import './popup.css';
import { mount } from 'svelte';
import Popup from './Popup.svelte';

// popup.html ships a static first frame (strip + skeleton) so the window paints in one
// frame; swap it for the live component in the same task, so there is no flash.
const target = document.getElementById('app')!;
target.replaceChildren();
mount(Popup, { target });
