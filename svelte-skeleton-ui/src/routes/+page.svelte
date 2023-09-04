<!-- YOU CAN DELETE EVERYTHING IN THIS PAGE -->
<script lang="ts">
	import { RangeSlider, tableSourceValues } from '@skeletonlabs/skeleton';
	import { RadioGroup, RadioItem } from '@skeletonlabs/skeleton';
	import { ProgressRadial, tableMapperValues } from '@skeletonlabs/skeleton';
	// import type { TableSource } from '@skeletonlabs/skeleton';
	import { onMount } from 'svelte';
	import { readable, writable } from 'svelte/store';

	
	interface RangeSliderProps {
		value: number;
		min: number;
		max: number;
		step: number;
		ticked: boolean;
	}

	// Define interfaces for your data
	interface EventData {
		time: string;
		action: string;
	}

	interface ChargeOptions{
			amps?: number;
			eco?: boolean;
			soc_limit?: number;
		
	}


	// Define interfaces for your data
	interface EventData {
		time: string;
		action: string;
	}

	interface RealTimeData {
		soc?: number;
		state?: string;
		temp?: number;
		fan?: number;
		amps?: number;
	}

	let socket: WebSocket; // Define socket here to make it accessible throughout the component
	let time = '';
	let eventData = writable<EventData[]>([]);
	let realTimeData = writable<RealTimeData>({});

	// subscribe to this and then update table
	if (typeof WebSocket !== 'undefined') {
		// This code will only run in the browser environment
		socket = new WebSocket('ws://10.0.1.177:5555');
		socket.addEventListener('open', () => {
			console.log('Opened');
			const message = JSON.stringify({ cmd: 'GetData' });

			// Send the message
			socket.send(message);
		});

		// This will be triggered when the WebSocket messages arrive
		socket.addEventListener('message', (event: MessageEvent) => {
			const message = JSON.parse(event.data);
			if (message.Events) {
				console.log(JSON.stringify(event.data));
				eventData.set(message.Events);
			}
			if (message.Data) {
				time = new Date().toLocaleTimeString();
				realTimeData.set(message.Data);
			}
		});
	}
	onMount(() => {
		document.getElementById('modeRadio')?.addEventListener('change', (event) => {
			const mode = value;
						const chargePayload = {
			cmd: {
				SetMode: mode
				},
			};
						console.log('Testing mode payload' + JSON.stringify(chargePayload));
			// Send the payload as a JSON string
    // {"cmd": {"SetMode": "V2h"}}
    // {"cmd": {"SetMode": "Idle"}}
			socket.send(JSON.stringify(chargePayload));
		});
		document.getElementById('chargeForm')?.addEventListener('submit', (event) => {
			event.preventDefault(); // Prevent the default form submission

			// Get form elements by their IDs
			const socRangeValue = Number(
				(document.getElementById('range-slider-amps') as HTMLInputElement)?.value
			);
			const ampsValue = Number(
				(document.getElementById('range-slider-soc') as HTMLInputElement)?.value
			);
			const ecoCheckbox = (document.getElementById('eco') as HTMLInputElement)?.checked;
		
			const chargePayload = {
			cmd: {
				SetMode: {
					Charge: {
						amps: ampsValue,
						eco: ecoCheckbox,
						soc_limit: socRangeValue,
						},	
					},
				},
			};

			console.log('Testing charge payload' + JSON.stringify(chargePayload));
			// Send the payload as a JSON string
			// {"cmd":{"SetMode":{"Charge":{"amps":90,"eco":false,"soc_limit":16}}}}
			socket.send(JSON.stringify(chargePayload));
		});
		console.log('on mount');
		if (socket) {
			async function fetchData() {
				try {
					let message = JSON.stringify({ cmd: 'GetData' });
					console.log('periodic: ' + message);
					// Send the message
					socket.send(message);

					message = JSON.stringify({ cmd: 'GetEvents' });
					console.log('periodic: ' + message);
					// Send the message
					socket.send(message);
				} catch (error) {
					console.error('WebSocket send error:', error);
				}
			}

			const interval = setInterval(fetchData, 3000);
			fetchData(); // Fetch data immediately when the component mounts

			return () => {
				console.log('onMount returned');
				clearInterval(interval);
				// You might also want to close the WebSocket connection here if needed
				// socket.close();
			};
		}
	});
	let amps_value = 16;
	let soc_range_value = 90;
	let value = '';
	let sourceData = [
		{ time: '00:01:59', action: 'Idle' },
		{ time: '00:01:59', action: 'Idle' }
	];

	const tableSimple = {
		head: ['Time', 'Action'],
		body: tableMapperValues(sourceData, ['time', 'action']),
		meta: tableMapperValues(sourceData, ['name', 'action'])
	};
</script>

<div class="container h-full mx-auto flex justify-center items-center">
	<div class="space-y-10 text-center flex flex-col items-center">
		<div class="place-self-center">
			<h2>Mode Selection</h2>
			<p>{value}</p>
			<RadioGroup id="modeRadio" rounded="rounded-container-token" display="flex-col">
				<RadioItem bind:group={value} name="justify" value="Idle">Idle</RadioItem>
				<RadioItem bind:group={value} name="justify" value="V2h">Load matching</RadioItem>
				<RadioItem bind:group={value} name="justify" value="Discharge">Discharge vehicle</RadioItem>
				<!-- <RadioItem bind:group={value} name="justify" value="Charge">Charge Vehicle</RadioItem> -->
			</RadioGroup>
		</div>
		<div class="place-self-center">
			<h2>Manual Charge Parameters</h2>
			<form id="chargeForm">
				<div class="grid container-fluid">
					<RangeSlider
						name="soc"
						id="range-slider-soc"
						bind:value={soc_range_value}
						min={30}
						max={100}
						step={1}
						ticked
					>
						<div class="flex justify-between items-center">
							<div class="font-bold">SoC</div>
							<div class="text-xs">{soc_range_value} / 100</div>
						</div>
					</RangeSlider>
					<RangeSlider
						name="amps"
						id="range-slider-amps"
						bind:value={amps_value}
						max={16}
						step={1}
						ticked
					>
						<div class="flex justify-between items-center">
							<div class="font-bold">Amps</div>
							<div class="text-xs">{amps_value} / 16</div>
						</div>
					</RangeSlider>

					<label for="eco" title="Permits charge to vehicle from exported energy only"
						>Solar Economy:
						<input type="checkbox" id="eco" name="eco" />
					</label>
				</div>
				<br />
				<button class="btn variant-filled" type="submit">Custom Charge</button>
			</form>
		</div>

		<h2>Event Table</h2>
		<table id="eventsTable">
			<thead>
				<tr>
					<th>Time</th>
					<th>Action</th>
					<th>Edit</th>
					<th>Delete</th>
				</tr>
			</thead>
			<tbody>
				{#each $eventData as event}
					<tr>
						<td>{event.time}</td>
						<td>{event.action}</td>
						<td><!-- Edit button here --></td>
						<td><!-- Delete button here --></td>
					</tr>
				{/each}
			</tbody>
		</table>
		<div class="grid container-fluid">
			<button id="addRowButton">Add Event</button>
			<button id="updateButton">Update</button>
		</div>

		<h2>Graphing</h2>
		<figure class="container-fluid">
			<div id="plotContainer" />
		</figure>
		<div class="data_section">
			<!-- <div id="connectionStatus">Attempting to establish WebSocket connection...</div> -->
			<h2>Realtime data</h2>
			<!-- {#each $realTimeData as data} -->
			<p>Localtime {time}</p>
			<p>SoC {$realTimeData.soc}%</p>
			<ProgressRadial value={$realTimeData.soc}>{$realTimeData.soc}%</ProgressRadial>
			<p>State {$realTimeData.state}</p>
			<p>Temperature {$realTimeData.temp}ºC</p>
			<p>Fan duty {$realTimeData.fan}%</p>
			<p>Amps {$realTimeData.amps}</p>
			<ProgressRadial value={Math.abs(($realTimeData.amps * 100) / 16)}
				>{Math.floor(Math.abs(($realTimeData.amps * 100) / 16))}%</ProgressRadial
			>
		</div>
	</div>
</div>

<style lang="postcss">
	figure {
		@apply flex relative flex-col;
	}
	figure svg,
	.img-bg {
		@apply w-64 h-64 md:w-80 md:h-80;
	}
	.img-bg {
		@apply absolute z-[-1] rounded-full blur-[50px] transition-all;
		animation: pulse 5s cubic-bezier(0, 0, 0, 0.5) infinite, glow 5s linear infinite;
	}
	@keyframes glow {
		0% {
			@apply bg-primary-400/50;
		}
		33% {
			@apply bg-secondary-400/50;
		}
		66% {
			@apply bg-tertiary-400/50;
		}
		100% {
			@apply bg-primary-400/50;
		}
	}
	@keyframes pulse {
		50% {
			transform: scale(1.5);
		}
	}
</style>
