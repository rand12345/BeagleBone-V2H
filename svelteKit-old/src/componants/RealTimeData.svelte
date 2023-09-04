<script>
    import { socket } from "./websocket.js";
    import { onMount } from "svelte";
    import { readable } from "svelte/store";

    // Create a readable store for real-time data
    let realTimeData = readable([], (set) => {
        // This will be triggered when the WebSocket messages arrive
        socket.addEventListener(
            "message",
            (/** @type {{ data: string; }} */ event) => {
                const message = JSON.parse(event.data);
                if (message.Data) {
                    set(message.Data);
                }
            }
        );
    });

    // Function to format timestamp to a human-readable date
    /**
     * @param {string | number | Date} timestamp
     */
    function formatTimestamp(timestamp) {
        return new Date(timestamp).toLocaleTimeString();
    }
</script>

<div id="data_section">
    <div>
        <h2>Realtime data</h2>
        {#each $realTimeData as data}
            <p>Localtime {formatTimestamp(data.date)}</p>
            <p>SoC {data.soc}%</p>
            <p>State {data.state}</p>
            <p>Temperature {data.temp}ºC</p>
            <p>Fan duty {data.fan}%</p>
            <p>Watts {data.dc_kw}</p>
        {/each}
    </div>
</div>
