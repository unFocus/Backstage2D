package backstage2d.render.stage3d 
{
	/**
	 * A bunch of booleans represented supported features for each layer/batch.
	 * Use this to tune render complexity, to reduce CPU and GPU load. On iPad1
	 * and iPhone4, you can gain an especially significant speed bump by skipping
	 * opacity and tinting calculations, so it's worth disabling those whenever
	 * you can. Note: These settings apply to every element withing a layer/batch.
	 * @author Kevin Newman
	 */
	public class BatchConfig 
	{
		public var opacity:Boolean = true;
		public var tinting:Boolean// = false;
		public var animation:Boolean// = false;
		public var useMips:Boolean = true;
		
		public function BatchConfig( options:Object ) 
		{
			for ( var key:String in options )
			{
				if ( key in this ) {
					if ( options[ key ] )
						this[ key ] = true;
					else
						this[ key ] = false;
				}
				else
					trace( "BatchConfig Warning: key [" + key + "] not a valid key" ); 
			}
		}
		
	}
}
